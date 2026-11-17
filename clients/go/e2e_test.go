package rusckersso

// End-to-end tests against a real provider started by
// clients/testing/provider.sh. Skipped unless RUSCKER_ISSUER is set:
//
//	source <workdir>/env && go test -v -run E2E ./...
//
// The browser is played by clients/testing/authorize.sh, which turns an
// authorization URL into the callback URL the provider redirects to.

import (
	"context"
	"errors"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
)

func e2eConfig(t *testing.T) Config {
	t.Helper()
	if os.Getenv("RUSCKER_ISSUER") == "" {
		t.Skip("RUSCKER_ISSUER is not set; source the env file written by clients/testing/provider.sh")
	}
	return Config{
		Issuer:       os.Getenv("RUSCKER_ISSUER"),
		ClientID:     os.Getenv("RUSCKER_CLIENT_ID"),
		ClientSecret: os.Getenv("RUSCKER_CLIENT_SECRET"),
		RedirectURI:  os.Getenv("RUSCKER_REDIRECT_URI"),
	}
}

func e2eClient(t *testing.T, cfg Config) *Client {
	t.Helper()
	c, err := New(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// authorize plays the browser: it returns the callback query.
func authorize(t *testing.T, authURL string) url.Values {
	t.Helper()
	script, _ := filepath.Abs("../testing/authorize.sh")
	out, err := exec.Command(script, authURL).Output()
	if err != nil {
		t.Fatalf("authorize.sh: %v", err)
	}
	cb, err := url.Parse(strings.TrimSpace(string(out)))
	if err != nil || cb.Query().Get("code") == "" {
		t.Fatalf("callback without a code: %q", out)
	}
	return cb.Query()
}

// countingTransport counts outgoing requests, to prove a refusal happened
// before any network call.
type countingTransport struct{ n atomic.Int64 }

func (c *countingTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	c.n.Add(1)
	return http.DefaultTransport.RoundTrip(r)
}

func TestE2EFullSignIn(t *testing.T) {
	c := e2eClient(t, e2eConfig(t))
	authURL, pending, err := c.StartSignIn()
	if err != nil {
		t.Fatal(err)
	}
	id, err := c.FinishSignIn(context.Background(), authorize(t, authURL), pending)
	if err != nil {
		t.Fatal(err)
	}
	if id.Sub == "" || id.Sub == "ana" {
		t.Errorf("sub should be a permanent id, got %q", id.Sub)
	}
	if id.Username != "ana" || id.Name != "Ana Maria de Souza" || id.Email != "ana@example.org" || id.Setor != "GAPE" {
		t.Errorf("identity = %+v", id)
	}
	if !slices.Equal(id.Groups, []string{"fiscal"}) {
		t.Errorf("groups = %v", id.Groups)
	}
	if len(id.GroupRoles) != 1 || !slices.Equal(id.Roles("fiscal"), []string{"analista", "gestor"}) {
		t.Errorf("group_roles = %v", id.GroupRoles)
	}
	if !id.HasGroup("fiscal") || !id.HasRole("fiscal", "gestor") || id.HasRole("fiscal", "admin") {
		t.Error("helpers")
	}
	if id.Claims["aud"] != c.oauth.ClientID {
		t.Errorf("raw claims = %v", id.Claims)
	}
}

func TestE2ETamperedStateRefusedBeforeNetwork(t *testing.T) {
	cfg := e2eConfig(t)
	counter := &countingTransport{}
	cfg.HTTPClient = &http.Client{Transport: counter}
	c := e2eClient(t, cfg)
	authURL, pending, _ := c.StartSignIn()
	q := authorize(t, authURL)
	before := counter.n.Load()
	q.Set("state", q.Get("state")+"x")
	if _, err := c.FinishSignIn(context.Background(), q, pending); !errors.Is(err, ErrStateMismatch) {
		t.Fatalf("err = %v", err)
	}
	if after := counter.n.Load(); after != before {
		t.Errorf("%d request(s) made after a state mismatch", after-before)
	}
}

func TestE2EReplayedCodeRefused(t *testing.T) {
	c := e2eClient(t, e2eConfig(t))
	authURL, pending, _ := c.StartSignIn()
	q := authorize(t, authURL)
	if _, err := c.FinishSignIn(context.Background(), q, pending); err != nil {
		t.Fatal(err)
	}
	_, err := c.FinishSignIn(context.Background(), q, pending)
	var te *TokenExchangeError
	if !errors.As(err, &te) || te.Code != "invalid_grant" {
		t.Fatalf("err = %v", err)
	}
}

func TestE2EWrongNonceRefused(t *testing.T) {
	c := e2eClient(t, e2eConfig(t))
	authURL, pending, _ := c.StartSignIn()
	q := authorize(t, authURL)
	pending.Nonce = "not-the-nonce-that-was-sent"
	_, err := c.FinishSignIn(context.Background(), q, pending)
	var ie *IDTokenError
	if !errors.As(err, &ie) || !strings.Contains(ie.Reason, "nonce") {
		t.Fatalf("err = %v", err)
	}
}

func TestE2EWrongSecretRefused(t *testing.T) {
	cfg := e2eConfig(t)
	good := e2eClient(t, cfg)
	cfg.ClientSecret = "not-the-secret"
	bad := e2eClient(t, cfg)
	authURL, pending, _ := good.StartSignIn()
	_, err := bad.FinishSignIn(context.Background(), authorize(t, authURL), pending)
	var te *TokenExchangeError
	if !errors.As(err, &te) || te.Code != "invalid_client" || te.StatusCode != http.StatusUnauthorized {
		t.Fatalf("err = %v", err)
	}
}

// A token issued to this client, verified by a client configured with
// another client_id, must be refused on aud. The exchange is done by the
// real client; only verification uses the other one.
func TestE2EWrongAudienceRefused(t *testing.T) {
	cfg := e2eConfig(t)
	c := e2eClient(t, cfg)
	cfg.ClientID = "another-system"
	other := e2eClient(t, cfg)
	authURL, pending, _ := c.StartSignIn()
	q := authorize(t, authURL)
	code, err := c.checkCallback(q, pending)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := c.exchange(context.Background(), code, pending.CodeVerifier)
	if err != nil {
		t.Fatal(err)
	}
	_, err = other.verifyIDToken(context.Background(), raw, pending.Nonce)
	var ie *IDTokenError
	if !errors.As(err, &ie) || !strings.Contains(ie.Reason, "audience") {
		t.Fatalf("err = %v", err)
	}
	// And the right client accepts the very same token.
	if _, err := c.verifyIDToken(context.Background(), raw, pending.Nonce); err != nil {
		t.Errorf("own client refused its token: %v", err)
	}
}
