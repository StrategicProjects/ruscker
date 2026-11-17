/**
 * The backend half of a Ruscker sign-in, for Node ≥ 20.
 *
 * Two ways to use it:
 *
 * - **Split flow** (a single-page frontend with its own backend): the
 *   browser half (`ruscker-sso/browser`) starts the sign-in and reads the
 *   return, then posts `{ code, codeVerifier, nonce }` here; the backend
 *   calls `exchangeCode`.
 * - **All-backend flow**: `startSignIn` on a `/login` route, keep
 *   `pending` in the session, `finishSignIn` on the `/callback` route.
 *
 * The ID token is verified with `jose` (JWKS + ES256); HTTP uses the
 * global `fetch`.
 */
import { timingSafeEqual } from "node:crypto";
import { createRemoteJWKSet, errors as joseErrors, jwtVerify } from "jose";
import {
  buildSignIn,
  checkCallback,
  toSearchParams,
  trimSlash,
  type CallbackInput,
  type PendingSignIn,
  type StartedSignIn,
} from "./core.js";
import { DiscoveryError, IdTokenError, IssuerMismatchError, SignInError, TokenExchangeError } from "./errors.js";
import { Identity } from "./identity.js";

export { DEFAULT_SCOPES, type CallbackInput, type PendingSignIn, type StartedSignIn } from "./core.js";
export * from "./errors.js";
export { Identity } from "./identity.js";

export interface SignInConfig {
  /** The portal's address, exactly as its discovery document states it, e.g. `https://portal.example.org`. */
  issuer: string;
  /** The system's id, as registered on the portal. */
  clientId: string;
  /** Shown once at registration. Keep it on the server. */
  clientSecret: string;
  /** The system's callback, exactly as registered. */
  redirectUri: string;
  /** Defaults to `openid profile email groups roles setor`. */
  scopes?: readonly string[];
  /** Milliseconds before a call to the portal is given up. Default 10 000. */
  timeoutMs?: number;
}

/** The parts of the discovery document this package uses. */
interface Discovery {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  jwks_uri: string;
}

/**
 * One configured client. Create it once at start-up and reuse it: it caches
 * the discovery document and the signing keys.
 */
export class SignInClient {
  /** The configuration, without the secret (so logging the client is safe). */
  readonly config: Readonly<Omit<SignInConfig, "clientSecret">>;
  /** A true private field: not enumerable, not printed by `console.log`. */
  readonly #clientSecret: string;
  private discovery?: Promise<Discovery>;
  private jwks?: ReturnType<typeof createRemoteJWKSet>;

  constructor(config: SignInConfig) {
    for (const key of ["issuer", "clientId", "clientSecret", "redirectUri"] as const) {
      if (!config[key]) throw new TypeError(`ruscker-sso: \`${key}\` is required`);
    }
    const { clientSecret, ...rest } = config;
    this.#clientSecret = clientSecret;
    this.config = rest;
  }

  /** The authorization endpoint, for a browser half that wants it from the backend. */
  async authorizationEndpoint(): Promise<string> {
    return (await this.discover()).authorization_endpoint;
  }

  /**
   * Build the authorization URL and the values to keep until the person
   * comes back. Store `pending` in the session, redirect to `url`.
   */
  async startSignIn(): Promise<StartedSignIn> {
    const d = await this.discover();
    return buildSignIn({
      authorizationEndpoint: d.authorization_endpoint,
      clientId: this.config.clientId,
      redirectUri: this.config.redirectUri,
      scopes: this.config.scopes,
    });
  }

  /**
   * The callback route. Runs the six checks in order: provider `error`,
   * `state`, `iss`, the code exchange, the ID token, then returns the
   * identity. The first three happen before any network call.
   *
   * `callback` is the callback URL, its query string, `URLSearchParams`,
   * or a parsed query object such as Express's `req.query`. Drop
   * `pending` from the session whatever the outcome: it is used once.
   */
  async finishSignIn(callback: CallbackInput, pending: PendingSignIn): Promise<Identity> {
    const code = checkCallback(toSearchParams(callback), pending, this.config.issuer, constantTimeEqual);
    return this.exchangeCode({ code, codeVerifier: pending.codeVerifier, nonce: pending.nonce });
  }

  /**
   * Split flow: the browser half already checked `state` and `iss` and
   * posted these three values. Exchange the code and verify the ID token.
   * A refused exchange is never retried: the code is spent.
   */
  async exchangeCode(input: { code: string; codeVerifier: string; nonce: string }): Promise<Identity> {
    // In the split flow these three come from a browser: refuse anything
    // that is not a non-empty string before using it.
    for (const key of ["code", "codeVerifier", "nonce"] as const) {
      if (typeof input?.[key] !== "string" || input[key] === "") {
        throw new TokenExchangeError(0, "invalid_request", `\`${key}\` must be a non-empty string`);
      }
    }
    const d = await this.discover();
    const body = new URLSearchParams({
      grant_type: "authorization_code",
      code: input.code,
      redirect_uri: this.config.redirectUri,
      code_verifier: input.codeVerifier,
    });
    // client_secret_basic: id and secret are form-encoded before base64 (RFC 6749 §2.3.1).
    const basic = btoa(`${encodeURIComponent(this.config.clientId)}:${encodeURIComponent(this.#clientSecret)}`);
    let res: Response;
    try {
      res = await fetch(d.token_endpoint, {
        method: "POST",
        headers: {
          authorization: `Basic ${basic}`,
          "content-type": "application/x-www-form-urlencoded",
          accept: "application/json",
        },
        body,
        // A redirect would re-send the code and verifier to another host.
        redirect: "error",
        signal: AbortSignal.timeout(this.timeout()),
      });
    } catch (e) {
      throw new TokenExchangeError(0, "", `the token endpoint could not be reached: ${(e as Error).message}`);
    }
    const json = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    if (!res.ok) {
      throw new TokenExchangeError(
        res.status,
        typeof json.error === "string" ? json.error : "",
        typeof json.error_description === "string" ? json.error_description : undefined,
      );
    }
    if (typeof json.id_token !== "string") throw new IdTokenError("the token response has no id_token");
    return this.verifyIdToken(json.id_token, input.nonce);
  }

  /**
   * Verify an ID token issued to THIS client: signature against the JWKS,
   * ES256 only (never taken from the token's header), `iss`, `aud` equal
   * to `clientId`, `exp`, and `nonce` equal to the one sent.
   */
  async verifyIdToken(idToken: string, nonce: string): Promise<Identity> {
    const d = await this.discover();
    // jose re-fetches the key set when it meets an unknown `kid` (key
    // rotation), at most once per 30 s cooldown.
    this.jwks ??= createRemoteJWKSet(new URL(d.jwks_uri), { timeoutDuration: this.timeout() });
    let payload: Record<string, unknown>;
    try {
      ({ payload } = await jwtVerify(idToken, this.jwks, {
        algorithms: ["ES256"],
        issuer: this.config.issuer,
        audience: this.config.clientId,
        requiredClaims: ["sub", "exp", "iat"],
      }));
    } catch (e) {
      if (e instanceof joseErrors.JOSEError) throw new IdTokenError(`${e.code}: ${e.message}`);
      throw e;
    }
    if (typeof payload.nonce !== "string" || !constantTimeEqual(payload.nonce, nonce)) {
      throw new IdTokenError("nonce mismatch");
    }
    // `sub` is the key applications store: it must be a real string.
    if (typeof payload.sub !== "string" || payload.sub === "") {
      throw new IdTokenError("the token has no subject (sub)");
    }
    return new Identity(payload);
  }

  /**
   * Read `{issuer}/.well-known/openid-configuration` once and keep it. A
   * failed read is not cached, so the next sign-in tries again.
   */
  private discover(): Promise<Discovery> {
    this.discovery ??= this.fetchDiscovery().catch((e: unknown) => {
      this.discovery = undefined;
      throw e;
    });
    return this.discovery;
  }

  private async fetchDiscovery(): Promise<Discovery> {
    const url = `${trimSlash(this.config.issuer)}/.well-known/openid-configuration`;
    let doc: Partial<Discovery>;
    try {
      const res = await fetch(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(this.timeout()) });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      doc = (await res.json()) as Partial<Discovery>;
    } catch (e) {
      throw new DiscoveryError(`could not read ${url}: ${(e as Error).message}`);
    }
    // Compared byte for byte: the provider writes this exact string into every token.
    if (doc.issuer !== this.config.issuer) throw new IssuerMismatchError(this.config.issuer, String(doc.issuer));
    for (const key of ["authorization_endpoint", "token_endpoint", "jwks_uri"] as const) {
      if (typeof doc[key] !== "string") throw new DiscoveryError(`the discovery document has no ${key}`);
    }
    return doc as Discovery;
  }

  private timeout(): number {
    return this.config.timeoutMs ?? 10_000;
  }
}

/** `state` and `nonce` comparison whose time does not depend on the content. */
function constantTimeEqual(a: string, b: string): boolean {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}
