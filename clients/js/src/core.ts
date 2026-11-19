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
  prompt?: "none";
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
  if (opts.prompt !== undefined) q.set("prompt", opts.prompt);
  return { url: url.toString(), pending };
}

/** What a sign-out can carry besides the client id, each optional. */
export interface SignOutOptions {
  /**
   * Where the portal sends the browser afterwards. Must be one of the
   * client's "Return addresses after sign-out" on the portal, exactly,
   * or the portal refuses the request.
   */
  postLogoutRedirectUri?: string;
  /** Echoed back on that return. At most 512 characters. */
  state?: string;
  /** The raw ID token, if the application kept it. */
  idTokenHint?: string;
}

/** The longest `state` the portal accepts on a sign-out. */
export const MAX_SIGN_OUT_STATE = 512;

/** Refuse a sign-out `state` the portal would refuse anyway. */
export function checkSignOutState(state: string | undefined): void {
  if (state !== undefined && state.length > MAX_SIGN_OUT_STATE) {
    throw new RangeError(`ruscker-sso: \`state\` is longer than ${MAX_SIGN_OUT_STATE} characters`);
  }
}

/**
 * The portal's sign-out address (OpenID Connect RP-Initiated Logout).
 * Query in the contract's order: `client_id` always, then
 * `id_token_hint`, `post_logout_redirect_uri` and `state` when given.
 * `URLSearchParams` does the form encoding.
 */
export function buildSignOutUrl(endSessionEndpoint: string, clientId: string, opts: SignOutOptions = {}): string {
  checkSignOutState(opts.state);
  const url = new URL(endSessionEndpoint);
  const q = url.searchParams;
  q.set("client_id", clientId);
  if (opts.idTokenHint !== undefined) q.set("id_token_hint", opts.idTokenHint);
  if (opts.postLogoutRedirectUri !== undefined) q.set("post_logout_redirect_uri", opts.postLogoutRedirectUri);
  if (opts.state !== undefined) q.set("state", opts.state);
  return url.toString();
}

/**
 * The portal's "change your password" page for this client. A fixed path
 * (it is not in discovery), so no network call: the portal signs the
 * person in if needed and links back to the client's System address.
 */
export function buildPasswordChangeUrl(issuer: string, clientId: string): string {
  return `${trimSlash(issuer)}/admin/account/password?${new URLSearchParams({ client_id: clientId })}`;
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
  // The portal always says who it is (RFC 9207). Requiring it means a
  // callback from anywhere else cannot be mistaken for the portal's.
  if (iss !== issuer) throw new IssuerMismatchError(issuer, iss ?? "(absent)");
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
