package rusckersso

// Unit tests: no network. The ID-token tests sign tokens locally with
// go-jose and verify them through go-oidc with a static key set, so the
// same verifier configuration as production is exercised.

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/coreos/go-oidc/v3/oidc"
	"github.com/go-jose/go-jose/v4"
	"golang.org/x/oauth2"
)

const (
	testIssuer   = "https://portal.example.org"
	testClientID = "my-system"
)

func testClient(verifier *oidc.IDTokenVerifier) *Client {
	return newClient(Config{
		Issuer: testIssuer, ClientID: testClientID, ClientSecret: "s3cret",
		RedirectURI: "https://app.example.org/auth/callback",
	}, oauth2.Endpoint{
		AuthURL:  testIssuer + "/oauth/authorize",
		TokenURL: testIssuer + "/oauth/token",
	}, verifier, http.DefaultClient)
}

func TestStartSignInURL(t *testing.T) {
	c := testClient(nil)
	raw, p, err := c.StartSignIn()
	if err != nil {
		t.Fatal(err)
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	if got := u.Scheme + "://" + u.Host + u.Path; got != testIssuer+"/oauth/authorize" {
		t.Errorf("endpoint = %s", got)
	}
	q := u.Query()
	want := map[string]string{
		"response_type":         "code",
		"client_id":             testClientID,
		"redirect_uri":          "https://app.example.org/auth/callback",
		"scope":                 "openid profile email groups roles setor",
		"code_challenge_method": "S256",
		"state":                 p.State,
		"nonce":                 p.Nonce,
	}
	for k, v := range want {
		if q.Get(k) != v {
			t.Errorf("%s = %q, want %q", k, q.Get(k), v)
		}
	}
	// The challenge is BASE64URL(SHA256(verifier)), unpadded.
	sum := sha256.Sum256([]byte(p.CodeVerifier))
	if got, want := q.Get("code_challenge"), base64.RawURLEncoding.EncodeToString(sum[:]); got != want {
		t.Errorf("code_challenge = %q, want %q", got, want)
	}
	if strings.Contains(raw, "client_secret") || strings.Contains(raw, "s3cret") {
		t.Error("the secret must never be in the authorization URL")
	}
}

func TestPendingValuesAreFreshAndLongEnough(t *testing.T) {
	c := testClient(nil)
	seen := map[string]bool{}
	for range 20 {
		_, p, err := c.StartSignIn()
		if err != nil {
			t.Fatal(err)
		}
		for _, v := range []string{p.State, p.Nonce, p.CodeVerifier} {
			// 32 random bytes → 43 base64url characters.
			if len(v) < 43 {
				t.Errorf("value %q is shorter than 43 characters", v)
			}
			if seen[v] {
				t.Errorf("value %q repeated", v)
			}
			seen[v] = true
		}
		if p.State == p.Nonce {
			t.Error("state and nonce must differ")
		}
	}
}

func TestPendingRoundTripsAsJSON(t *testing.T) {
	_, p, _ := testClient(nil).StartSignIn()
	b, err := json.Marshal(p)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), `"code_verifier"`) {
		t.Errorf("JSON = %s", b)
	}
	var back Pending
	if err := json.Unmarshal(b, &back); err != nil || back != p {
		t.Errorf("round trip = %+v, %v", back, err)
	}
}

func TestScopesAlwaysIncludeOpenID(t *testing.T) {
	if got := withOpenID([]string{"profile"}); strings.Join(got, " ") != "openid profile" {
		t.Errorf("got %v", got)
	}
	if got := withOpenID([]string{"profile", "openid"}); len(got) != 2 {
		t.Errorf("got %v", got)
	}
}

// The callback checks run before any network: a nil verifier and an
// unroutable token URL prove FinishSignIn returns before using either.
func TestCallbackChecks(t *testing.T) {
	c := testClient(nil)
	p := Pending{State: "the-state", Nonce: "n", CodeVerifier: strings.Repeat("v", 43)}
	cases := []struct {
		name  string
		query url.Values
		check func(error) bool
	}{
		{"provider error", url.Values{"error": {"access_denied"}, "state": {"the-state"}}, func(err error) bool {
			var pe *ProviderError
			return errors.Is(err, ErrProvider) && errors.As(err, &pe) && pe.Code == "access_denied"
		}},
		{"error wins even with a bad state", url.Values{"error": {"invalid_scope"}, "state": {"forged"}}, func(err error) bool {
			return errors.Is(err, ErrProvider)
		}},
		{"state mismatch", url.Values{"code": {"c"}, "state": {"forged"}}, func(err error) bool {
			return errors.Is(err, ErrStateMismatch)
		}},
		{"state missing", url.Values{"code": {"c"}}, func(err error) bool {
			return errors.Is(err, ErrStateMismatch)
		}},
		{"issuer mismatch", url.Values{"code": {"c"}, "state": {"the-state"}, "iss": {"https://evil.example.org"}}, func(err error) bool {
			return errors.Is(err, ErrIssuerMismatch)
		}},
		{"no code", url.Values{"state": {"the-state"}, "iss": {testIssuer}}, func(err error) bool {
			return errors.Is(err, ErrMissingCode)
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := c.FinishSignIn(context.Background(), tc.query, p)
			if !tc.check(err) {
				t.Errorf("unexpected error %v", err)
			}
		})
	}
	// An empty stored state never matches, even an empty returned one.
	if _, err := c.FinishSignIn(context.Background(), url.Values{"code": {"c"}, "state": {""}}, Pending{}); !errors.Is(err, ErrStateMismatch) {
		t.Errorf("empty pending: %v", err)
	}
	// iss absent is accepted (the check is "if present").
	if code, err := c.checkCallback(url.Values{"code": {"c"}, "state": {"the-state"}}, p); err != nil || code != "c" {
		t.Errorf("no iss: %q, %v", code, err)
	}
}

// --- ID-token verification, with locally signed tokens ---

type signer struct {
	key crypto.Signer
	kid string
	alg jose.SignatureAlgorithm
}

func newES256(t *testing.T) signer {
	k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return signer{key: k, kid: "k1", alg: jose.ES256}
}

func (s signer) sign(t *testing.T, claims map[string]any) string {
	opts := (&jose.SignerOptions{}).WithType("JWT").WithHeader("kid", s.kid)
	sg, err := jose.NewSigner(jose.SigningKey{Algorithm: s.alg, Key: s.key}, opts)
	if err != nil {
		t.Fatal(err)
	}
	payload, _ := json.Marshal(claims)
	jws, err := sg.Sign(payload)
	if err != nil {
		t.Fatal(err)
	}
	out, err := jws.CompactSerialize()
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func (s signer) verifier(clientID string) *oidc.IDTokenVerifier {
	keys := &oidc.StaticKeySet{PublicKeys: []crypto.PublicKey{s.key.Public()}}
	return oidc.NewVerifier(testIssuer, keys, verifierConfig(clientID))
}

func baseClaims() map[string]any {
	now := time.Now()
	return map[string]any{
		"iss": testIssuer, "aud": testClientID, "sub": "5966f47e-a879-4aee-8588-69e44be72525",
		"iat": now.Unix(), "exp": now.Add(5 * time.Minute).Unix(), "nonce": "the-nonce",
		"preferred_username": "ana", "name": "Ana Maria de Souza", "email": "ana@example.org",
		"setor": "GAPE", "groups": []string{"fiscal"},
		"group_roles": map[string][]string{"fiscal": {"analista", "gestor"}},
	}
}

func TestVerifyIDTokenHappyPath(t *testing.T) {
	s := newES256(t)
	id, err := testClient(s.verifier(testClientID)).verifyIDToken(context.Background(), s.sign(t, baseClaims()), "the-nonce")
	if err != nil {
		t.Fatal(err)
	}
	if id.Sub != "5966f47e-a879-4aee-8588-69e44be72525" || id.Username != "ana" || id.Name != "Ana Maria de Souza" ||
		id.Email != "ana@example.org" || id.Setor != "GAPE" {
		t.Errorf("identity = %+v", id)
	}
	if !id.HasGroup("fiscal") || id.HasGroup("other") {
		t.Error("HasGroup")
	}
	if !id.HasRole("fiscal", "gestor") || id.HasRole("fiscal", "admin") || id.HasRole("other", "gestor") {
		t.Error("HasRole")
	}
	if got := strings.Join(id.Roles("fiscal"), ","); got != "analista,gestor" {
		t.Errorf("Roles = %s", got)
	}
	if id.Roles("other") != nil {
		t.Error("Roles of an unknown group should be nil")
	}
	if id.Claims["iss"] != testIssuer {
		t.Errorf("raw claims = %v", id.Claims)
	}
}

func TestVerifyIDTokenRefusals(t *testing.T) {
	s := newES256(t)
	mutate := func(f func(map[string]any)) string {
		c := baseClaims()
		f(c)
		return s.sign(t, c)
	}
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	rsTok := signer{key: rsaKey, kid: "k1", alg: jose.RS256}.sign(t, baseClaims())
	other := newES256(t) // same kid, another key

	cases := []struct {
		name, token, nonce, reason string
	}{
		{"wrong nonce", s.sign(t, baseClaims()), "another-nonce", "nonce"},
		{"no nonce in token", mutate(func(c map[string]any) { delete(c, "nonce") }), "the-nonce", "nonce"},
		{"wrong audience", mutate(func(c map[string]any) { c["aud"] = "another-system" }), "the-nonce", "audience"},
		{"wrong issuer", mutate(func(c map[string]any) { c["iss"] = "https://evil.example.org" }), "the-nonce", "issuer"},
		{"expired", mutate(func(c map[string]any) { c["exp"] = time.Now().Add(-time.Hour).Unix() }), "the-nonce", "expired"},
		{"RS256 refused", rsTok, "the-nonce", "ES256"},
		{"signed by another key", other.sign(t, baseClaims()), "the-nonce", "signature"},
	}
	c := testClient(s.verifier(testClientID))
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := c.verifyIDToken(context.Background(), tc.token, tc.nonce)
			var ie *IDTokenError
			if !errors.Is(err, ErrIDTokenInvalid) || !errors.As(err, &ie) {
				t.Fatalf("err = %v", err)
			}
			if !strings.Contains(ie.Reason, tc.reason) {
				t.Errorf("reason = %q, want it to mention %q (err: %v)", ie.Reason, tc.reason, err)
			}
		})
	}
}

func TestMissingOptionalClaims(t *testing.T) {
	id, err := identityFromClaims(map[string]any{"sub": "x", "preferred_username": "bob", "name": "bob"})
	if err != nil {
		t.Fatal(err)
	}
	if id.Email != "" || id.Setor != "" {
		t.Errorf("identity = %+v", id)
	}
	if id.Groups == nil || len(id.Groups) != 0 || id.GroupRoles == nil || len(id.GroupRoles) != 0 {
		t.Errorf("groups/roles should be empty, not nil: %+v", id)
	}
	if id.HasGroup("fiscal") || id.HasRole("fiscal", "gestor") || id.Roles("fiscal") != nil {
		t.Error("helpers on an empty identity")
	}
}

func TestTokenExchangeErrorKeepsTheCode(t *testing.T) {
	err := error(&TokenExchangeError{Code: "invalid_grant", StatusCode: 400})
	var te *TokenExchangeError
	if !errors.Is(err, ErrTokenExchange) || !errors.As(err, &te) || te.Code != "invalid_grant" {
		t.Errorf("err = %v", err)
	}
}
