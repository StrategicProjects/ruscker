/**
 * The pieces both halves share: random values, the PKCE challenge, the
 * authorization URL and the first three checks of a callback.
 *
 * Only standard Web APIs are used here (`crypto.getRandomValues`,
 * `crypto.subtle`, `URL`), so this file runs unchanged in a browser and in
 * Node ≥ 20. Nothing in it is cryptography of our own: the randomness and
 * the SHA-256 come from the platform.
 */
import { IssuerMismatchError, ProviderError, SignInError, StateMismatchError } from "./errors.js";

/** The scopes asked for when none are given: everything the provider offers. */
export const DEFAULT_SCOPES = ["openid", "profile", "email", "groups", "roles", "setor"] as const;

/**
 * The three values to keep between "send the person to the portal" and
 * "the person came back". A plain JSON-serialisable object, so it can go
 * into a server-side session, a short-lived `HttpOnly` cookie or
 * `sessionStorage` as is.
 */
export interface PendingSignIn {
  state: string;
  nonce: string;
  /** The PKCE secret; its SHA-256 went to the portal as `code_challenge`. */
  codeVerifier: string;
}

/** What starting a sign-in produces: where to send the person, and what to keep. */
export interface StartedSignIn {
  url: string;
  pending: PendingSignIn;
}

/**
 * 32 bytes from the platform's cryptographic random source, base64url
 * encoded (43 characters of `A-Z a-z 0-9 - _`). That alphabet is valid for
 * all three uses: `state`, `nonce` and the PKCE verifier (which must be
 * 43–128 characters of `A-Z a-z 0-9 - . _ ~`).
 */
export function randomValue(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return base64url(bytes);
}

/** The PKCE `S256` challenge: base64url(SHA-256(verifier)), from Web Crypto. */
export async function codeChallenge(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return base64url(new Uint8Array(digest));
}

function base64url(bytes: Uint8Array): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * Generate fresh state/nonce/verifier and build the authorization URL.
 * `response_type=code` and `code_challenge_method=S256` are fixed: the
 * provider accepts nothing else.
 */
export async function buildSignIn(opts: {
  authorizationEndpoint: string;
  clientId: string;
  redirectUri: string;
  scopes?: readonly string[];
}): Promise<StartedSignIn> {
  const pending: PendingSignIn = {
    state: randomValue(),
    nonce: randomValue(),
    codeVerifier: randomValue(),
  };
  const url = new URL(opts.authorizationEndpoint);
  const q = url.searchParams;
  q.set("response_type", "code");
  q.set("client_id", opts.clientId);
  q.set("redirect_uri", opts.redirectUri);
  q.set("scope", (opts.scopes ?? DEFAULT_SCOPES).join(" "));
  q.set("state", pending.state);
  q.set("nonce", pending.nonce);
  q.set("code_challenge", await codeChallenge(pending.codeVerifier));
  q.set("code_challenge_method", "S256");
  return { url: url.toString(), pending };
}

/** Anything a callback query can arrive as. */
export type CallbackInput = string | URL | URLSearchParams | Record<string, string | string[] | undefined>;

/**
 * Normalise the callback into `URLSearchParams`. A string may be a full
 * URL (`https://app.example.org/callback?code=…`) or just its query
 * (`?code=…` or `code=…`); an object is e.g. Express's `req.query`.
 */
export function toSearchParams(input: CallbackInput): URLSearchParams {
  if (input instanceof URLSearchParams) return input;
  if (input instanceof URL) return input.searchParams;
  if (typeof input === "string") {
    if (/^[a-z][a-z0-9+.-]*:/i.test(input)) return new URL(input).searchParams;
    return new URLSearchParams(input.startsWith("?") ? input.slice(1) : input);
  }
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(input)) {
    if (typeof value === "string") params.append(key, value);
    else if (Array.isArray(value)) for (const v of value) params.append(key, v);
  }
  return params;
}

/**
 * Checks 1–3 of the contract, in order, all without any network call:
 * an `error` parameter, then `state`, then `iss`. Returns the code.
 *
 * `equal` is the string comparison to use for `state`: the server passes a
 * constant-time one from `node:crypto`.
 */
export function checkCallback(
  params: URLSearchParams,
  pending: PendingSignIn,
  issuer: string,
  equal: (a: string, b: string) => boolean,
): string {
  const error = params.get("error");
  if (error !== null) {
    throw new ProviderError(error, params.get("error_description") ?? undefined);
  }
  // A parameter repeated in the query is an attack or a bug; refuse it.
  if (params.getAll("state").length !== 1) throw new StateMismatchError();
  const state = params.get("state") ?? "";
  if (!pending.state || !equal(state, pending.state)) throw new StateMismatchError();
  const iss = params.get("iss");
  if (iss !== null && iss !== issuer) throw new IssuerMismatchError(issuer, iss);
  const codes = params.getAll("code");
  if (codes.length !== 1 || !codes[0]) {
    throw new SignInError("the callback carries no authorization code");
  }
  return codes[0];
}

/** Drop a trailing slash so `https://portal.example.org/` and `…org` compare as written by the provider. */
export function trimSlash(s: string): string {
  return s.endsWith("/") ? s.slice(0, -1) : s;
}
