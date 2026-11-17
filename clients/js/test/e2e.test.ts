// End-to-end against a real provider started by clients/testing/provider.sh.
// Skipped unless RUSCKER_ISSUER is set (source the provider's env file).
import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import * as browser from "../src/browser.js";
import { IdTokenError, SignInClient, StateMismatchError, TokenExchangeError } from "../src/server.js";

const env = process.env;
const skip = env.RUSCKER_ISSUER ? false : "RUSCKER_ISSUER is not set (source the env file of clients/testing/provider.sh)";

// test/ → build-test/test/ after compilation; authorize.sh is in clients/testing/.
const AUTHORIZE = fileURLToPath(new URL("../../../testing/authorize.sh", import.meta.url));

/** Play the signed-in person's browser: return the callback URL the provider redirects to. */
function authorize(url: string): string {
  return execFileSync(AUTHORIZE, [url], { encoding: "utf8" }).trim();
}

function config(over: Partial<ConstructorParameters<typeof SignInClient>[0]> = {}) {
  return new SignInClient({
    issuer: env.RUSCKER_ISSUER!,
    clientId: env.RUSCKER_CLIENT_ID!,
    clientSecret: env.RUSCKER_CLIENT_SECRET!,
    redirectUri: env.RUSCKER_REDIRECT_URI!,
    ...over,
  });
}

function assertAna(id: Awaited<ReturnType<SignInClient["exchangeCode"]>>) {
  assert.match(id.sub, /^[0-9a-f-]{36}$/);
  assert.equal(id.username, "ana");
  assert.equal(id.name, "Ana Maria de Souza");
  assert.equal(id.email, "ana@example.org");
  assert.equal(id.setor, "GAPE");
  assert.deepEqual(id.groups, ["fiscal"]);
  assert.deepEqual(id.groupRoles, { fiscal: ["analista", "gestor"] });
  assert.ok(id.hasGroup("fiscal") && !id.hasGroup("outro"));
  assert.ok(id.hasRole("fiscal", "gestor"));
  assert.deepEqual(id.roles("outro"), []);
  assert.equal(id.claims.aud, env.RUSCKER_CLIENT_ID);
}

describe("end to end", { skip }, () => {
  test("all-backend flow: a full sign-in returns the identity", async () => {
    const c = config();
    const { url, pending } = await c.startSignIn();
    const callback = authorize(url);
    assert.ok(callback.startsWith(env.RUSCKER_REDIRECT_URI!), callback);
    assertAna(await c.finishSignIn(callback, pending));
  });

  test("split flow: browser half starts and reads, backend exchanges", async () => {
    const storage = new Map<string, string>();
    const store = { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => void storage.set(k, v), removeItem: (k: string) => void storage.delete(k) };
    let went = "";
    await browser.redirectToSignIn(
      { issuer: env.RUSCKER_ISSUER!, clientId: env.RUSCKER_CLIENT_ID!, redirectUri: env.RUSCKER_REDIRECT_URI! },
      { storage: store, assign: (u) => (went = u) },
    );
    const posted = browser.readCallback({ storage: store, url: authorize(went) });
    assertAna(await config().exchangeCode(posted));
  });

  test("a tampered state is refused before any network call", async () => {
    const c = config();
    const { url, pending } = await c.startSignIn();
    const callback = new URL(authorize(url));
    callback.searchParams.set("state", "tampered");
    const realFetch = globalThis.fetch;
    let calls = 0;
    globalThis.fetch = ((...a: Parameters<typeof fetch>) => (calls++, realFetch(...a))) as typeof fetch;
    try {
      await assert.rejects(c.finishSignIn(callback, pending), StateMismatchError);
    } finally {
      globalThis.fetch = realFetch;
    }
    assert.equal(calls, 0);
  });

  test("a replayed code is refused", async () => {
    const c = config();
    const { url, pending } = await c.startSignIn();
    const callback = authorize(url);
    await c.finishSignIn(callback, pending);
    await assert.rejects(
      c.finishSignIn(callback, pending),
      (e: unknown) => e instanceof TokenExchangeError && e.error === "invalid_grant",
    );
  });

  test("a wrong nonce is refused", async () => {
    const c = config();
    const { url, pending } = await c.startSignIn();
    await assert.rejects(
      c.finishSignIn(authorize(url), { ...pending, nonce: "not-the-nonce-sent" }),
      (e: unknown) => e instanceof IdTokenError && e.reason === "nonce mismatch",
    );
  });

  test("a wrong client secret is refused", async () => {
    const { url, pending } = await config().startSignIn();
    const callback = authorize(url);
    await assert.rejects(
      config({ clientSecret: "0".repeat(64) }).finishSignIn(callback, pending),
      (e: unknown) => e instanceof TokenExchangeError && e.status === 401 && e.error === "invalid_client",
    );
    // The provider documents that a failed client authentication does not
    // spend the code: the real client can still redeem it.
    assertAna(await config().finishSignIn(callback, pending));
  });

  test("an ID token issued to this client is refused by another client (aud)", async () => {
    const c = config();
    const { url, pending } = await c.startSignIn();
    const code = new URL(authorize(url)).searchParams.get("code")!;
    // Redeem by hand to get the raw ID token, then verify it as another system would.
    const res = await fetch(`${env.RUSCKER_ISSUER}/oauth/token`, {
      method: "POST",
      headers: { authorization: `Basic ${btoa(`${env.RUSCKER_CLIENT_ID}:${env.RUSCKER_CLIENT_SECRET}`)}` },
      body: new URLSearchParams({ grant_type: "authorization_code", code, redirect_uri: env.RUSCKER_REDIRECT_URI!, code_verifier: pending.codeVerifier }),
    });
    assert.equal(res.status, 200);
    const { id_token } = (await res.json()) as { id_token: string };
    // The same token verifies for its own client…
    assertAna(await c.verifyIdToken(id_token, pending.nonce));
    // …and not for another one.
    await assert.rejects(
      config({ clientId: "another-system" }).verifyIdToken(id_token, pending.nonce),
      (e: unknown) => e instanceof IdTokenError && /aud/.test(e.reason),
    );
  });
});
