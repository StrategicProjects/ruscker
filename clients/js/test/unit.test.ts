// Unit tests: no network. The server half talks to a fake portal that
// lives inside this file (a stubbed global `fetch` and a local ES256 key).
import { test, describe, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { SignJWT, exportJWK, generateKeyPair, type JWK } from "jose";

import * as browser from "../src/browser.js";
import {
  Identity,
  IdTokenError,
  IssuerMismatchError,
  ProviderError,
  SignInClient,
  StateMismatchError,
  TokenExchangeError,
  DiscoveryError,
} from "../src/server.js";

const ISSUER = "https://portal.example.org";
const REDIRECT = "https://app.example.org/callback";

function memoryStore(): browser.KeyValueStore & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

const b64url = (buf: Buffer) => buf.toString("base64url");

describe("starting a sign-in", () => {
  test("the URL carries the fixed profile and fresh values", async () => {
    const { url, pending } = await browser.startSignIn({ issuer: ISSUER, clientId: "demo", redirectUri: REDIRECT });
    const u = new URL(url);
    assert.equal(u.origin + u.pathname, `${ISSUER}/oauth/authorize`);
    const q = u.searchParams;
    assert.equal(q.get("response_type"), "code");
    assert.equal(q.get("code_challenge_method"), "S256");
    assert.equal(q.get("client_id"), "demo");
    assert.equal(q.get("redirect_uri"), REDIRECT);
    assert.equal(q.get("scope"), "openid profile email groups roles setor");
    assert.equal(q.get("state"), pending.state);
    assert.equal(q.get("nonce"), pending.nonce);
    // S256 = base64url(sha256(verifier)), computed here independently.
    assert.equal(q.get("code_challenge"), b64url(createHash("sha256").update(pending.codeVerifier).digest()));
    assert.equal(q.get("code_challenge")!.length, 43);
  });

  test("state, nonce and verifier are 32 random bytes, distinct, verifier-safe", async () => {
    const seen = new Set<string>();
    for (let i = 0; i < 20; i++) {
      const { pending } = await browser.startSignIn({ issuer: ISSUER, clientId: "demo", redirectUri: REDIRECT });
      for (const v of [pending.state, pending.nonce, pending.codeVerifier]) {
        assert.match(v, /^[A-Za-z0-9_-]{43}$/);
        assert.equal(Buffer.from(v, "base64url").length, 32);
        assert.ok(!seen.has(v), "a value repeated");
        seen.add(v);
      }
    }
  });

  test("custom scopes and authorization endpoint", async () => {
    const { url } = await browser.startSignIn({
      issuer: ISSUER, clientId: "demo", redirectUri: REDIRECT,
      scopes: ["openid", "groups"], authorizationEndpoint: "https://portal.example.org/other/authorize",
    });
    const u = new URL(url);
    assert.equal(u.pathname, "/other/authorize");
    assert.equal(u.searchParams.get("scope"), "openid groups");
  });

  test("redirectToSignIn stores the pending values and navigates", async () => {
    const storage = memoryStore();
    let went = "";
    await browser.redirectToSignIn({ issuer: ISSUER, clientId: "demo", redirectUri: REDIRECT }, { storage, assign: (u) => (went = u) });
    const stored = JSON.parse(storage.getItem(browser.STORAGE_KEY)!);
    assert.equal(new URL(went).searchParams.get("state"), stored.state);
    assert.equal(stored.issuer, ISSUER);
  });
});

describe("browser readCallback", () => {
  async function started() {
    const storage = memoryStore();
    let went = "";
    await browser.redirectToSignIn({ issuer: ISSUER, clientId: "demo", redirectUri: REDIRECT }, { storage, assign: (u) => (went = u) });
    const state = new URL(went).searchParams.get("state")!;
    const stored = JSON.parse(storage.getItem(browser.STORAGE_KEY)!);
    return { storage, state, stored };
  }

  test("returns code + verifier + nonce and clears the store", async () => {
    const { storage, state, stored } = await started();
    const r = browser.readCallback({ storage, url: `${REDIRECT}?code=abc&state=${state}&iss=${encodeURIComponent(ISSUER)}` });
    assert.deepEqual(r, { code: "abc", codeVerifier: stored.codeVerifier, nonce: stored.nonce });
    assert.equal(storage.getItem(browser.STORAGE_KEY), null);
  });

  test("a tampered state is refused, and the store is still cleared", async () => {
    const { storage } = await started();
    assert.throws(() => browser.readCallback({ storage, url: `${REDIRECT}?code=abc&state=forged` }), StateMismatchError);
    assert.equal(storage.getItem(browser.STORAGE_KEY), null);
  });

  test("a missing state is refused", async () => {
    const { storage } = await started();
    assert.throws(() => browser.readCallback({ storage, url: `${REDIRECT}?code=abc` }), StateMismatchError);
  });

  test("an error callback is reported with its code", async () => {
    const { storage, state } = await started();
    assert.throws(
      () => browser.readCallback({ storage, url: `${REDIRECT}?error=access_denied&state=${state}` }),
      (e: unknown) => e instanceof ProviderError && e.error === "access_denied",
    );
  });

  test("an issuer mismatch is refused", async () => {
    const { storage, state } = await started();
    assert.throws(
      () => browser.readCallback({ storage, url: `${REDIRECT}?code=abc&state=${state}&iss=https%3A%2F%2Fevil.example.org` }),
      IssuerMismatchError,
    );
  });

  test("no pending sign-in", () => {
    assert.throws(() => browser.readCallback({ storage: memoryStore(), url: `${REDIRECT}?code=abc&state=x` }), browser.NoPendingSignInError);
  });
});

// ---------------------------------------------------------------- server

/** A fake portal: discovery, JWKS and a token endpoint answering `tokenReply`. */
interface FakePortal {
  calls: string[];
  tokenReply: () => { status: number; body: unknown };
  lastTokenRequest?: { headers: Headers; body: URLSearchParams };
  sign: (claims: Record<string, unknown>, opts?: { alg?: string }) => Promise<string>;
}

const realFetch = globalThis.fetch;
let portal: FakePortal;

async function installPortal(discoveryIssuer = ISSUER): Promise<FakePortal> {
  const { privateKey, publicKey } = await generateKeyPair("ES256", { extractable: true });
  const jwk: JWK = { ...(await exportJWK(publicKey)), kid: "k1", alg: "ES256", use: "sig" };
  const p: FakePortal = {
    calls: [],
    tokenReply: () => ({ status: 500, body: {} }),
    sign: (claims, opts) =>
      new SignJWT(claims)
        .setProtectedHeader({ alg: opts?.alg ?? "ES256", kid: "k1", typ: "JWT" })
        .sign(privateKey),
  };
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input instanceof Request ? input.url : input);
    p.calls.push(url);
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    if (url === `${ISSUER}/.well-known/openid-configuration`) {
      return json(200, {
        issuer: discoveryIssuer,
        authorization_endpoint: `${ISSUER}/oauth/authorize`,
        token_endpoint: `${ISSUER}/oauth/token`,
        jwks_uri: `${ISSUER}/oauth/jwks`,
      });
    }
    if (url === `${ISSUER}/oauth/jwks`) return json(200, { keys: [jwk] });
    if (url === `${ISSUER}/oauth/token`) {
      p.lastTokenRequest = { headers: new Headers(init?.headers), body: new URLSearchParams(String(init?.body)) };
      const r = p.tokenReply();
      return json(r.status, r.body);
    }
    return json(404, {});
  }) as typeof fetch;
  return p;
}

function client(clientId = "demo") {
  return new SignInClient({ issuer: ISSUER, clientId, clientSecret: "s3cret", redirectUri: REDIRECT });
}

function claims(extra: Record<string, unknown> = {}) {
  const now = Math.floor(Date.now() / 1000);
  return { iss: ISSUER, aud: "demo", sub: "uuid-1", iat: now, exp: now + 300, nonce: "n1", ...extra };
}

describe("server", () => {
  beforeEach(async () => {
    portal = await installPortal();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  test("startSignIn uses the discovered endpoint", async () => {
    const { url } = await client().startSignIn();
    assert.ok(url.startsWith(`${ISSUER}/oauth/authorize?`));
  });

  test("discovery is cached", async () => {
    const c = client();
    await c.startSignIn();
    await c.startSignIn();
    assert.equal(portal.calls.filter((u) => u.includes("openid-configuration")).length, 1);
  });

  test("a discovery issuer that differs is refused", async () => {
    portal = await installPortal("https://portal.example.org/");
    await assert.rejects(client().startSignIn(), IssuerMismatchError);
  });

  test("state mismatch, error and iss are checked before any network call", async () => {
    const c = client();
    const pending = { state: "S", nonce: "n1", codeVerifier: "v".repeat(43) };
    await assert.rejects(c.finishSignIn("?code=x&state=T", pending), StateMismatchError);
    await assert.rejects(c.finishSignIn("?code=x&state=S&state=S", pending), StateMismatchError);
    await assert.rejects(
      c.finishSignIn({ error: "access_denied", error_description: "not in group" }, pending),
      (e: unknown) => e instanceof ProviderError && e.error === "access_denied" && e.description === "not in group",
    );
    await assert.rejects(c.finishSignIn(`${REDIRECT}?code=x&state=S&iss=https%3A%2F%2Fother.example.org`, pending), IssuerMismatchError);
    assert.deepEqual(portal.calls, []);
  });

  test("a full sign-in: Basic auth, verifier, identity", async () => {
    portal.tokenReply = () => ({ status: 200, body: { id_token: tokenHolder.value } });
    const tokenHolder = { value: await portal.sign(claims({
      preferred_username: "ana", name: "Ana", email: "ana@example.org", setor: "X",
      groups: ["g1"], group_roles: { g1: ["r1", "r2"] },
    })) };
    const pending = { state: "S", nonce: "n1", codeVerifier: "v".repeat(43) };
    const id = await client().finishSignIn(`${REDIRECT}?code=C&state=S&iss=${encodeURIComponent(ISSUER)}`, pending);
    const req = portal.lastTokenRequest!;
    assert.equal(req.headers.get("authorization"), `Basic ${Buffer.from("demo:s3cret").toString("base64")}`);
    assert.equal(req.body.get("code"), "C");
    assert.equal(req.body.get("code_verifier"), pending.codeVerifier);
    assert.equal(req.body.get("redirect_uri"), REDIRECT);
    assert.equal(req.body.get("grant_type"), "authorization_code");
    assert.equal(req.body.get("client_secret"), null, "never both auth methods");
    assert.equal(id.sub, "uuid-1");
    assert.equal(id.username, "ana");
    assert.ok(id.hasGroup("g1"));
    assert.ok(id.hasRole("g1", "r2"));
  });

  test("a refused exchange carries the provider's error code", async () => {
    portal.tokenReply = () => ({ status: 400, body: { error: "invalid_grant", error_description: "used" } });
    await assert.rejects(
      client().exchangeCode({ code: "C", codeVerifier: "v".repeat(43), nonce: "n1" }),
      (e: unknown) => e instanceof TokenExchangeError && e.status === 400 && e.error === "invalid_grant",
    );
  });

  test("ID token checks: nonce, aud, iss, exp, algorithm", async () => {
    const c = client();
    const reason = async (token: string, nonce = "n1") => {
      try {
        await c.verifyIdToken(token, nonce);
        return "accepted";
      } catch (e) {
        assert.ok(e instanceof IdTokenError, String(e));
        return e.reason;
      }
    };
    assert.equal(await reason(await portal.sign(claims())), "accepted");
    assert.equal(await reason(await portal.sign(claims()), "other"), "nonce mismatch");
    assert.equal(await reason(await portal.sign(claims({ nonce: undefined }))), "nonce mismatch");
    assert.match(await reason(await portal.sign(claims({ aud: "someone-else" }))), /aud/);
    assert.match(await reason(await portal.sign(claims({ iss: "https://other.example.org" }))), /iss/);
    assert.match(await reason(await portal.sign(claims({ exp: Math.floor(Date.now() / 1000) - 60 }))), /exp|expired/);
    // A token signed with another algorithm is refused however it is labelled.
    const hs = await new SignJWT(claims()).setProtectedHeader({ alg: "HS256", kid: "k1" }).sign(new TextEncoder().encode("x".repeat(32)));
    assert.match(await reason(hs), /alg/i);
    assert.match(await reason("not.a.jwt"), /.+/);
  });

  test("the token response without id_token is an ID token error", async () => {
    portal.tokenReply = () => ({ status: 200, body: { access_token: "a" } });
    await assert.rejects(client().exchangeCode({ code: "C", codeVerifier: "v".repeat(43), nonce: "n1" }), IdTokenError);
  });

  test("an unreachable portal is a DiscoveryError, and is retried next time", async () => {
    globalThis.fetch = (async () => { throw new Error("down"); }) as typeof fetch;
    const c = client();
    await assert.rejects(c.startSignIn(), DiscoveryError);
    portal = await installPortal();
    await c.startSignIn();
  });

  test("missing configuration is refused up front", () => {
    assert.throws(() => new SignInClient({ issuer: ISSUER, clientId: "demo", clientSecret: "", redirectUri: REDIRECT }), TypeError);
  });
});

describe("identity", () => {
  test("helpers", () => {
    const id = new Identity({ sub: "u", groups: ["fiscal"], group_roles: { fiscal: ["analista", "gestor"] } });
    assert.ok(id.hasGroup("fiscal"));
    assert.ok(!id.hasGroup("outro"));
    assert.ok(id.hasRole("fiscal", "gestor"));
    assert.ok(!id.hasRole("fiscal", "admin"));
    assert.deepEqual(id.roles("fiscal"), ["analista", "gestor"]);
    assert.deepEqual(id.roles("outro"), []);
    assert.deepEqual(id.roles("constructor"), [], "no prototype leakage");
  });

  test("missing optional claims", () => {
    const id = new Identity({ sub: "u" });
    assert.equal(id.email, undefined);
    assert.equal(id.setor, undefined);
    assert.equal(id.username, undefined);
    assert.deepEqual(id.groups, []);
    assert.deepEqual(id.groupRoles, {});
    assert.deepEqual(id.claims, { sub: "u" });
  });
});
