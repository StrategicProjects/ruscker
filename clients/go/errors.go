package rusckersso

import (
	"errors"
	"fmt"
)

// Sentinel errors, for errors.Is. The typed errors below also match
// their sentinel, so both styles work:
//
//	if errors.Is(err, rusckersso.ErrProvider) { … }
//	var pe *rusckersso.ProviderError
//	if errors.As(err, &pe) && pe.Code == "access_denied" { … }
var (
	// ErrProvider: the provider sent the person back with ?error=….
	ErrProvider = errors.New("rusckersso: the provider refused the sign-in")
	// ErrStateMismatch: the callback's state is not the one stored for
	// this browser — a forged or stale callback, or a lost Pending.
	ErrStateMismatch = errors.New("rusckersso: state does not match")
	// ErrIssuerMismatch: the callback's iss names another provider.
	ErrIssuerMismatch = errors.New("rusckersso: iss does not match the issuer")
	// ErrMissingCode: the callback has neither an error nor a code.
	ErrMissingCode = errors.New("rusckersso: the callback has no code")
	// ErrTokenExchange: the token endpoint refused the code or could not
	// be reached.
	ErrTokenExchange = errors.New("rusckersso: token exchange failed")
	// ErrIDTokenInvalid: the ID token failed verification.
	ErrIDTokenInvalid = errors.New("rusckersso: ID token is not valid")
)

// ProviderError is an ?error=… callback. Code is the provider's code,
// e.g. "access_denied" (the person is not in the system's groups).
type ProviderError struct {
	Code        string
	Description string
}

func (e *ProviderError) Error() string {
	if e.Description != "" {
		return fmt.Sprintf("rusckersso: the provider refused the sign-in: %s (%s)", e.Code, e.Description)
	}
	return "rusckersso: the provider refused the sign-in: " + e.Code
}

// Is makes errors.Is(err, ErrProvider) true.
func (e *ProviderError) Is(target error) bool { return target == ErrProvider }

// TokenExchangeError is a refused (or failed) code exchange. Code is the
// provider's "error" field — "invalid_grant" (code used, expired or not
// matching) or "invalid_client" (wrong secret) — and empty when the
// endpoint could not be reached at all.
type TokenExchangeError struct {
	Code        string
	Description string
	StatusCode  int
	Err         error
}

func (e *TokenExchangeError) Error() string {
	if e.Code != "" {
		return fmt.Sprintf("rusckersso: token exchange refused: %s (HTTP %d)", e.Code, e.StatusCode)
	}
	return fmt.Sprintf("rusckersso: token exchange failed: %v", e.Err)
}

// Is makes errors.Is(err, ErrTokenExchange) true.
func (e *TokenExchangeError) Is(target error) bool { return target == ErrTokenExchange }

// Unwrap exposes the underlying x/oauth2 or network error.
func (e *TokenExchangeError) Unwrap() error { return e.Err }

// IDTokenError is an ID token that failed verification. Reason is a short
// human-readable cause ("nonce does not match the one sent", "audience is
// not this client", …); Err, when set, is go-oidc's original error.
type IDTokenError struct {
	Reason string
	Err    error
}

func (e *IDTokenError) Error() string {
	if e.Err != nil {
		return fmt.Sprintf("rusckersso: ID token is not valid: %s: %v", e.Reason, e.Err)
	}
	return "rusckersso: ID token is not valid: " + e.Reason
}

// Is makes errors.Is(err, ErrIDTokenInvalid) true.
func (e *IDTokenError) Is(target error) bool { return target == ErrIDTokenInvalid }

// Unwrap exposes go-oidc's original error.
func (e *IDTokenError) Unwrap() error { return e.Err }
