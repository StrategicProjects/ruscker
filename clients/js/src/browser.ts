/**
 * The browser half of a Ruscker sign-in ("split flow").
 *
 * The browser starts the sign-in and reads the return, but it never
 * exchanges the code: the portal's clients are confidential, so the token
 * endpoint wants the client secret, and a secret shipped to a browser is
 * public. The page posts `{ code, codeVerifier, nonce }` to its OWN
 * backend, which finishes with `exchangeCode` from `ruscker-sso/server`.
 *
 * No dependencies; only Web Crypto, `URL` and `sessionStorage`.
 */
import { buildSignIn, checkCallback, trimSlash, type PendingSignIn, type StartedSignIn } from "./core.js";
import { NoPendingSignInError } from "./errors.js";

export { DEFAULT_SCOPES, type PendingSignIn, type StartedSignIn } from "./core.js";
export * from "./errors.js";

export interface BrowserSignInOptions {
  /** The portal's address, exactly as its discovery document states it, e.g. `https://portal.example.org`. */
  issuer: string;
  clientId: string;
  /** The callback page, exactly as registered on the portal. */
  redirectUri: string;
  /** Defaults to `openid profile email groups roles setor`. */
  scopes?: readonly string[];
  /**
   * Where to send the person. Defaults to `{issuer}/oauth/authorize`, the
   * provider's fixed path. The browser cannot read the discovery document
   * itself (it is served without CORS headers), so pass the backend's
   * `authorizationEndpoint()` here if you prefer to take it from there.
   */
  authorizationEndpoint?: string;
}

/** What the callback page hands to its backend. */
export interface CallbackResult {
  code: string;
  codeVerifier: string;
  nonce: string;
}

/** The subset of `Storage` used, so tests can pass a plain object. */
export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** The key under which the pending sign-in is kept in `sessionStorage`. */
export const STORAGE_KEY = "ruscker-sso:pending";

/** What is stored: the pending values plus the issuer they belong to. */
interface StoredSignIn extends PendingSignIn {
  issuer: string;
}

/**
 * Build the authorization URL and the values to keep. Does not store or
 * navigate anything — use `redirectToSignIn` for that, or store
 * `pending` yourself.
 */
export function startSignIn(opts: BrowserSignInOptions): Promise<StartedSignIn> {
  return buildSignIn({
    authorizationEndpoint: opts.authorizationEndpoint ?? `${trimSlash(opts.issuer)}/oauth/authorize`,
    clientId: opts.clientId,
    redirectUri: opts.redirectUri,
    scopes: opts.scopes,
  });
}

/**
 * Start a sign-in, keep its values in `sessionStorage` (per tab, gone
 * when the tab closes) and send the browser to the portal.
 *
 * `env` exists for tests; a page leaves it out.
 */
export async function redirectToSignIn(
  opts: BrowserSignInOptions,
  env: { storage?: KeyValueStore; assign?: (url: string) => void } = {},
): Promise<void> {
  const { url, pending } = await startSignIn(opts);
  const stored: StoredSignIn = { ...pending, issuer: opts.issuer };
  (env.storage ?? sessionStorage).setItem(STORAGE_KEY, JSON.stringify(stored));
  (env.assign ?? ((u: string) => window.location.assign(u)))(url);
}

/**
 * On the callback page: read `code`/`state`/`iss`/`error` from the URL,
 * check `state` and `iss` against what `redirectToSignIn` stored, clear
 * the store (whatever the outcome — a pending sign-in is used once) and
 * return what the backend needs.
 *
 * Throws `ProviderError`, `StateMismatchError`, `IssuerMismatchError` or
 * `NoPendingSignInError`. Consider `history.replaceState` afterwards so
 * the code does not linger in the address bar.
 */
export function readCallback(
  env: { storage?: KeyValueStore; url?: string | URL } = {},
): CallbackResult {
  const storage = env.storage ?? sessionStorage;
  const raw = storage.getItem(STORAGE_KEY);
  storage.removeItem(STORAGE_KEY);
  const params = new URL(env.url ?? window.location.href).searchParams;

  let stored: StoredSignIn | undefined;
  try {
    stored = raw ? (JSON.parse(raw) as StoredSignIn) : undefined;
  } catch {
    stored = undefined;
  }
  // An error from the provider is worth reporting even without a stored
  // sign-in (e.g. the tab was reloaded), so check it first.
  if (!stored?.state || !stored.nonce || !stored.codeVerifier) {
    if (params.has("error")) checkCallback(params, { state: "", nonce: "", codeVerifier: "" }, "", sameString);
    throw new NoPendingSignInError();
  }
  const code = checkCallback(params, stored, stored.issuer, sameString);
  return { code, codeVerifier: stored.codeVerifier, nonce: stored.nonce };
}

/**
 * A comparison whose time does not depend on where the strings differ.
 * Browsers offer no built-in one; `state` is a per-tab random value, so
 * this is belt and braces rather than a load-bearing defence.
 */
function sameString(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/**
 * A plain "Sign in with …" button: no framework, no brand assets, a little
 * inline styling the page's CSS can override.
 */
export function signInButton(opts: { label: string; onClick: (ev: MouseEvent) => void }): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = opts.label;
  button.style.cssText =
    "display:inline-flex;align-items:center;gap:.5em;padding:.6em 1.2em;" +
    "border:1px solid currentColor;border-radius:.4em;background:transparent;" +
    "font:inherit;cursor:pointer;";
  button.addEventListener("click", opts.onClick);
  return button;
}
