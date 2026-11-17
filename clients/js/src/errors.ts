/**
 * The ways a sign-in can fail, as separate classes so an application can
 * tell "the person said no" from "someone tampered with the return" from
 * "our configuration is wrong" with a plain `instanceof`.
 *
 * Every class extends `SignInError`, so `catch (e) { if (e instanceof
 * SignInError) … }` catches them all.
 */

/** Base class of every error this package throws on purpose. */
export class SignInError extends Error {
  constructor(message: string) {
    super(message);
    this.name = new.target.name;
  }
}

/**
 * The provider sent the person back with `?error=…` instead of a code,
 * e.g. `access_denied` when they are not in the system's allowed groups.
 */
export class ProviderError extends SignInError {
  constructor(
    /** The provider's `error` code, e.g. `access_denied`. */
    readonly error: string,
    /** The provider's `error_description`, when it sent one. */
    readonly description?: string,
  ) {
    super(`the provider refused the sign-in: ${error}${description ? ` (${description})` : ""}`);
  }
}

/**
 * The callback's `state` is missing or differs from the stored one: the
 * return does not belong to the sign-in this browser started (a forged or
 * replayed link). Checked before any network call.
 */
export class StateMismatchError extends SignInError {
  constructor() {
    super("the callback's state does not match the stored sign-in");
  }
}

/**
 * An `iss` (in the callback, or in the discovery document) is not the
 * configured issuer: the answer comes from another provider.
 */
export class IssuerMismatchError extends SignInError {
  constructor(
    readonly expected: string,
    readonly received: string,
  ) {
    super(`issuer mismatch: expected ${expected}, received ${received}`);
  }
}

/**
 * No pending sign-in was found (browser half): the page was opened
 * directly, in another tab, or the stored values were already used.
 */
export class NoPendingSignInError extends SignInError {
  constructor() {
    super("no pending sign-in was found in this browser");
  }
}

/**
 * The token endpoint refused the code. `error` is the provider's code:
 * `invalid_client` (wrong secret, disabled client) or `invalid_grant`
 * (code used, expired, other redirect URI or verifier). Never retry with
 * the same code: it is spent.
 */
export class TokenExchangeError extends SignInError {
  constructor(
    /** HTTP status of the token endpoint's answer (0 when it could not be reached). */
    readonly status: number,
    /** The provider's `error` code, or `""` when the answer had none. */
    readonly error: string,
    readonly description?: string,
  ) {
    super(`the token exchange was refused (${status} ${error || "no error code"})${description ? `: ${description}` : ""}`);
  }
}

/** The ID token failed verification; `reason` says which check. */
export class IdTokenError extends SignInError {
  constructor(
    /** Short reason, e.g. `nonce mismatch`, `"aud" claim check failed`. */
    readonly reason: string,
  ) {
    super(`the ID token is invalid: ${reason}`);
  }
}

/** The discovery document could not be read or is not usable. */
export class DiscoveryError extends SignInError {}
