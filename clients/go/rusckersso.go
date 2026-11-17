// Package rusckersso signs people in to a Go backend through a Ruscker
// portal ("Sign in with Ruscker"), using OpenID Connect.
//
// It is a thin wrapper: discovery, JWKS handling and ID-token signature
// checks come from github.com/coreos/go-oidc, the code exchange and PKCE
// from golang.org/x/oauth2. What this package adds is the provider's
// profile already set (authorization code + PKCE S256, ES256 only,
// client secret in an HTTP Basic header) and the checks a library leaves
// to the application: the callback's state and iss, and the token's nonce.
//
// The flow is two calls:
//
//	url, pending, err := client.StartSignIn()            // keep pending, redirect to url
//	identity, err := client.FinishSignIn(ctx, r.URL.Query(), pending)
//
// After that the application issues its own session; this package keeps
// no state between the two calls.
package rusckersso

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/coreos/go-oidc/v3/oidc"
	"golang.org/x/oauth2"
)

// DefaultScopes is what a sign-in asks for when Config.Scopes is empty:
// every claim the provider knows how to give.
var DefaultScopes = []string{"openid", "profile", "email", "groups", "roles", "setor"}

// Config is what the portal's administrator gives the system's developers.
type Config struct {
	// Issuer is the portal's address exactly as its discovery document
	// states it, e.g. "https://portal.example.org" (include the subpath
	// when the portal is mounted under one). It is compared byte for byte.
	Issuer string
	// ClientID is the system's id, as registered on the portal.
	ClientID string
	// ClientSecret is the secret shown once at registration. Server only.
	ClientSecret string
	// RedirectURI is the system's callback, exactly as registered.
	RedirectURI string
	// Scopes defaults to DefaultScopes. "openid" is added if missing.
	Scopes []string
	// HTTPClient is used for discovery, keys and the token exchange.
	// Optional. The default has a 10-second timeout and does not follow
	// redirects: a stalled portal must not hang the application, and a
	// redirect must not carry the code to another host.
	HTTPClient *http.Client
}

// String keeps the client secret out of logs: fmt prints a Config with
// this method, whatever the verb.
func (c Config) String() string {
	return fmt.Sprintf("rusckersso.Config{Issuer: %q, ClientID: %q, ClientSecret: [redacted], RedirectURI: %q, Scopes: %q}",
		c.Issuer, c.ClientID, c.RedirectURI, c.Scopes)
}

// String keeps the client secret out of logs.
func (c *Client) String() string {
	return fmt.Sprintf("rusckersso.Client{Issuer: %q, ClientID: %q}", c.issuer, c.oauth.ClientID)
}

// ClockLeeway is how far the application's clock may run ahead of the
// portal's before a token counts as expired. Every package uses 60 s.
const ClockLeeway = 60 * time.Second

// defaultHTTPClient is used when Config.HTTPClient is nil.
func defaultHTTPClient() *http.Client {
	return &http.Client{
		Timeout: 10 * time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
}

// Client is safe for concurrent use; create one at startup and share it.
type Client struct {
	issuer   string
	oauth    oauth2.Config
	verifier *oidc.IDTokenVerifier
	http     *http.Client
}

// Pending holds the three values that must survive between StartSignIn
// and FinishSignIn. Keep it where only this person's browser session can
// reach it (a server-side session, or a short-lived HttpOnly cookie —
// signed or encrypted if your framework offers it). The JSON tags let it
// be stored as one value.
type Pending struct {
	State        string `json:"state"`
	Nonce        string `json:"nonce"`
	CodeVerifier string `json:"code_verifier"`
}

// New reads the provider's discovery document once and returns a client.
// go-oidc refuses a discovery document whose issuer differs from
// cfg.Issuer, so a mistyped issuer fails here, at startup.
func New(ctx context.Context, cfg Config) (*Client, error) {
	if cfg.Issuer == "" || cfg.ClientID == "" || cfg.ClientSecret == "" || cfg.RedirectURI == "" {
		return nil, errors.New("rusckersso: Issuer, ClientID, ClientSecret and RedirectURI are all required")
	}
	httpClient := cfg.HTTPClient
	if httpClient == nil {
		httpClient = defaultHTTPClient()
	}
	// The provider object keeps the JWKS cache for the client's lifetime;
	// it must not be bound to a request context that ends, hence
	// context.WithoutCancel for the HTTP client only.
	providerCtx := oidc.ClientContext(context.WithoutCancel(ctx), httpClient)
	provider, err := oidc.NewProvider(providerCtx, cfg.Issuer)
	if err != nil {
		return nil, fmt.Errorf("rusckersso: reading the discovery document: %w", err)
	}
	return newClient(cfg, provider.Endpoint(), provider.VerifierContext(providerCtx, verifierConfig(cfg.ClientID)), httpClient), nil
}

// verifierConfig pins the algorithm: ES256 is the only one accepted, and
// it is never read from the token itself. go-oidc then checks the
// signature (refreshing the JWKS once on an unknown key id), iss, aud and exp.
func verifierConfig(clientID string) *oidc.Config {
	return &oidc.Config{
		ClientID:             clientID,
		SupportedSigningAlgs: []string{oidc.ES256},
		// The same 60 seconds of clock tolerance as the other packages:
		// go-oidc has no leeway setting, so "now" is read a minute early
		// when it checks the expiry.
		Now: func() time.Time { return time.Now().Add(-ClockLeeway) },
	}
}

// newClient is split from New so unit tests can build a client without a
// network (verifier may then be nil; those tests never reach it).
func newClient(cfg Config, endpoint oauth2.Endpoint, verifier *oidc.IDTokenVerifier, httpClient *http.Client) *Client {
	// The provider accepts both Basic and body authentication; fixing one
	// avoids x/oauth2's "try one, then the other" probing, which would
	// present the code twice.
	endpoint.AuthStyle = oauth2.AuthStyleInHeader
	return &Client{
		issuer: cfg.Issuer,
		oauth: oauth2.Config{
			ClientID:     cfg.ClientID,
			ClientSecret: cfg.ClientSecret,
			RedirectURL:  cfg.RedirectURI,
			Endpoint:     endpoint,
			Scopes:       withOpenID(cfg.Scopes),
		},
		verifier: verifier,
		http:     httpClient,
	}
}

func withOpenID(scopes []string) []string {
	if len(scopes) == 0 {
		return append([]string(nil), DefaultScopes...)
	}
	for _, s := range scopes {
		if s == "openid" {
			return append([]string(nil), scopes...)
		}
	}
	return append([]string{"openid"}, scopes...)
}

// StartSignIn returns the URL to redirect the browser to, and the values
// to keep until the person comes back. Each value is fresh, from
// crypto/rand, for every call.
func (c *Client) StartSignIn() (string, Pending, error) {
	state, err := randomToken()
	if err != nil {
		return "", Pending{}, err
	}
	nonce, err := randomToken()
	if err != nil {
		return "", Pending{}, err
	}
	p := Pending{State: state, Nonce: nonce, CodeVerifier: oauth2.GenerateVerifier()}
	// AuthCodeURL always sets response_type=code; S256ChallengeOption adds
	// code_challenge and code_challenge_method=S256.
	authURL := c.oauth.AuthCodeURL(p.State, oidc.Nonce(p.Nonce), oauth2.S256ChallengeOption(p.CodeVerifier))
	return authURL, p, nil
}

// randomToken is 32 random bytes, URL-safe base64 (43 characters).
func randomToken() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("rusckersso: reading random bytes: %w", err)
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}

// FinishSignIn completes the sign-in from the callback's query string and
// the stored Pending. The checks run in this order, and the first three
// happen before any network call:
//
//  1. an "error" parameter → *ProviderError
//  2. state must equal pending.State → ErrStateMismatch
//  3. iss must be present and equal the issuer → ErrIssuerMismatch
//  4. the code is exchanged (secret in Basic, redirect_uri, code_verifier)
//     → *TokenExchangeError on refusal
//  5. the ID token is verified (ES256, signature, iss, aud, exp, nonce)
//     → *IDTokenError
//  6. the identity is returned.
//
// A code is single-use: on failure, start a new sign-in, never retry.
func (c *Client) FinishSignIn(ctx context.Context, callback url.Values, pending Pending) (*Identity, error) {
	code, err := c.checkCallback(callback, pending)
	if err != nil {
		return nil, err
	}
	rawIDToken, err := c.exchange(ctx, code, pending.CodeVerifier)
	if err != nil {
		return nil, err
	}
	return c.verifyIDToken(ctx, rawIDToken, pending.Nonce)
}

// checkCallback runs steps 1–3: pure, no network.
func (c *Client) checkCallback(q url.Values, pending Pending) (string, error) {
	for _, name := range []string{"error", "state", "iss", "code"} {
		if len(q[name]) > 1 {
			return "", ErrRepeatedParameter
		}
	}
	if code := q.Get("error"); code != "" {
		return "", &ProviderError{Code: code, Description: q.Get("error_description")}
	}
	// An empty stored state would make an empty returned state "match":
	// treat a lost or never-set Pending as a mismatch.
	if pending.State == "" || !equal(q.Get("state"), pending.State) {
		return "", ErrStateMismatch
	}
	// The portal always says who it is (RFC 9207). Requiring it means a
	// callback from anywhere else cannot be mistaken for the portal's.
	if !equal(q.Get("iss"), c.issuer) {
		return "", ErrIssuerMismatch
	}
	code := q.Get("code")
	if code == "" {
		return "", ErrMissingCode
	}
	return code, nil
}

// equal compares in constant time, so the comparison does not reveal how
// much of a guessed value was right.
func equal(a, b string) bool {
	return subtle.ConstantTimeCompare([]byte(a), []byte(b)) == 1
}

// exchange runs step 4 and returns the raw ID token.
func (c *Client) exchange(ctx context.Context, code, verifier string) (string, error) {
	ctx = context.WithValue(ctx, oauth2.HTTPClient, c.http)
	tok, err := c.oauth.Exchange(ctx, code, oauth2.VerifierOption(verifier))
	if err != nil {
		var re *oauth2.RetrieveError
		if errors.As(err, &re) {
			status := 0
			if re.Response != nil {
				status = re.Response.StatusCode
			}
			return "", &TokenExchangeError{Code: re.ErrorCode, Description: re.ErrorDescription, StatusCode: status, Err: err}
		}
		// Network failure, timeout, unreadable response.
		return "", &TokenExchangeError{Err: err}
	}
	raw, _ := tok.Extra("id_token").(string)
	if raw == "" {
		return "", &IDTokenError{Reason: "the token response has no id_token"}
	}
	return raw, nil
}

// verifyIDToken runs step 5 and builds the identity.
func (c *Client) verifyIDToken(ctx context.Context, raw, nonce string) (*Identity, error) {
	ctx = oidc.ClientContext(ctx, c.http)
	tok, err := c.verifier.Verify(ctx, raw)
	if err != nil {
		return nil, &IDTokenError{Reason: reasonFor(err), Err: err}
	}
	// go-oidc leaves the nonce to the caller.
	if nonce == "" || !equal(tok.Nonce, nonce) {
		return nil, &IDTokenError{Reason: "nonce does not match the one sent"}
	}
	var claims map[string]any
	if err := tok.Claims(&claims); err != nil {
		return nil, &IDTokenError{Reason: "claims are not readable", Err: err}
	}
	id, err := identityFromClaims(claims)
	if err != nil {
		return nil, &IDTokenError{Reason: "claims are not readable", Err: err}
	}
	return id, nil
}

// reasonFor turns go-oidc's errors into a short reason. go-oidc exposes
// typed errors only for expiry; the others are matched on their text,
// with the original error always kept in IDTokenError.Err.
func reasonFor(err error) string {
	var expired *oidc.TokenExpiredError
	if errors.As(err, &expired) {
		return "token expired"
	}
	msg := err.Error()
	switch {
	case strings.Contains(msg, "expected audience"):
		return "audience is not this client"
	case strings.Contains(msg, "id token issued by a different provider"):
		return "issuer does not match"
	case strings.Contains(msg, "signing algorithm"), strings.Contains(msg, "unexpected signature algorithm"):
		return "signing algorithm is not ES256"
	case strings.Contains(msg, "signature"):
		return "signature is not valid"
	default:
		return "token is not valid"
	}
}
