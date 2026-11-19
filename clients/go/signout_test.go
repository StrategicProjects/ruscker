package rusckersso

// Unit tests for the sign-out and password addresses and the prompt
// option. Discovery is served by a local httptest server: New reads it
// once, and SignOutURL must not fetch it again.

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"

	"golang.org/x/oauth2"
)

// discoveryServer serves a discovery document whose issuer is the
// server's own address; endSession is announced when withEndSession.
// The returned counter tells how many times the document was read.
func discoveryServer(t *testing.T, withEndSession bool) (*httptest.Server, *atomic.Int32) {
	t.Helper()
	var hits atomic.Int32
	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/.well-known/openid-configuration" {
			http.NotFound(w, r)
			return
		}
		hits.Add(1)
		doc := map[string]any{
			"issuer":                                srv.URL,
			"authorization_endpoint":                srv.URL + "/oauth/authorize",
			"token_endpoint":                        srv.URL + "/oauth/token",
			"jwks_uri":                              srv.URL + "/oauth/jwks",
			"id_token_signing_alg_values_supported": []string{"ES256"},
		}
		if withEndSession {
			doc["end_session_endpoint"] = srv.URL + "/oauth/logout"
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(doc)
	}))
	t.Cleanup(srv.Close)
	return srv, &hits
}

func newFromServer(t *testing.T, srv *httptest.Server) *Client {
	t.Helper()
	c, err := New(context.Background(), Config{
		Issuer: srv.URL, ClientID: "my system", ClientSecret: "s3cret",
		RedirectURI: "https://app.example.org/auth/callback",
	})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestSignOutURLOrderAndEncoding(t *testing.T) {
	srv, hits := discoveryServer(t, true)
	c := newFromServer(t, srv)
	got, err := c.SignOutURL(SignOut{
		PostLogoutRedirectURI: "https://app.example.org/signed-out?x=1",
		State:                 "a b&c?d",
		IDTokenHint:           "eyJ.tok.en",
	})
	if err != nil {
		t.Fatal(err)
	}
	want := srv.URL + "/oauth/logout?client_id=my+system&id_token_hint=eyJ.tok.en" +
		"&post_logout_redirect_uri=https%3A%2F%2Fapp.example.org%2Fsigned-out%3Fx%3D1" +
		"&state=a+b%26c%3Fd"
	if got != want {
		t.Errorf("got  %s\nwant %s", got, want)
	}
	// The encoding round-trips through a standard parser.
	u, _ := url.Parse(got)
	if u.Query().Get("state") != "a b&c?d" {
		t.Errorf("state decodes to %q", u.Query().Get("state"))
	}
	// Discovery was read once, in New; SignOutURL did not fetch it again.
	if _, err := c.SignOutURL(SignOut{}); err != nil {
		t.Fatal(err)
	}
	if n := hits.Load(); n != 1 {
		t.Errorf("discovery read %d times, want 1", n)
	}
}

func TestSignOutURLOnlyClientIDWhenNothingGiven(t *testing.T) {
	srv, _ := discoveryServer(t, true)
	got, err := newFromServer(t, srv).SignOutURL(SignOut{})
	if err != nil {
		t.Fatal(err)
	}
	if want := srv.URL + "/oauth/logout?client_id=my+system"; got != want {
		t.Errorf("got %s, want %s", got, want)
	}
	// One optional field alone: client_id still first, the others absent.
	c := testClient(nil)
	c.endSession = testIssuer + "/oauth/logout"
	got, _ = c.SignOutURL(SignOut{State: "s"})
	if want := testIssuer + "/oauth/logout?client_id=my-system&state=s"; got != want {
		t.Errorf("got %s, want %s", got, want)
	}
}

func TestSignOutURLStateLimit(t *testing.T) {
	c := testClient(nil)
	c.endSession = testIssuer + "/oauth/logout"
	if _, err := c.SignOutURL(SignOut{State: strings.Repeat("x", 512)}); err != nil {
		t.Errorf("512 characters must be accepted: %v", err)
	}
	// Characters, not bytes: 512 two-byte characters are still fine.
	if _, err := c.SignOutURL(SignOut{State: strings.Repeat("é", 512)}); err != nil {
		t.Errorf("512 non-ASCII characters must be accepted: %v", err)
	}
	if _, err := c.SignOutURL(SignOut{State: strings.Repeat("x", 513)}); !errors.Is(err, ErrStateTooLong) {
		t.Errorf("513 characters: got %v, want ErrStateTooLong", err)
	}
}

func TestSignOutURLWithoutEndSessionEndpoint(t *testing.T) {
	srv, _ := discoveryServer(t, false)
	_, err := newFromServer(t, srv).SignOutURL(SignOut{})
	if !errors.Is(err, ErrDiscovery) || !strings.Contains(err.Error(), "end_session_endpoint") {
		t.Errorf("got %v, want ErrDiscovery naming end_session_endpoint", err)
	}
}

func TestNewDiscoveryFailureMatchesErrDiscovery(t *testing.T) {
	srv := httptest.NewServer(http.NotFoundHandler())
	defer srv.Close()
	_, err := New(context.Background(), Config{Issuer: srv.URL, ClientID: "a", ClientSecret: "b", RedirectURI: "c"})
	if !errors.Is(err, ErrDiscovery) {
		t.Errorf("got %v, want ErrDiscovery", err)
	}
}

func TestPasswordChangeURL(t *testing.T) {
	if got, want := testClient(nil).PasswordChangeURL(), "https://portal.example.org/admin/account/password?client_id=my-system"; got != want {
		t.Errorf("got %s, want %s", got, want)
	}
	c := newClient(Config{Issuer: "https://portal.example.org/sub", ClientID: "a b&c", ClientSecret: "s", RedirectURI: "r"}, oauthEndpointForTest(), nil, http.DefaultClient)
	if got, want := c.PasswordChangeURL(), "https://portal.example.org/sub/admin/account/password?client_id=a+b%26c"; got != want {
		t.Errorf("got %s, want %s", got, want)
	}
}

func TestPromptOnlyWhenAsked(t *testing.T) {
	c := testClient(nil)
	plain, _, err := c.StartSignIn()
	if err != nil {
		t.Fatal(err)
	}
	if u, _ := url.Parse(plain); u.Query().Has("prompt") {
		t.Errorf("prompt must be absent by default: %s", plain)
	}
	silent, _, err := c.StartSignIn(WithPrompt("none"))
	if err != nil {
		t.Fatal(err)
	}
	u, _ := url.Parse(silent)
	if got := u.Query()["prompt"]; len(got) != 1 || got[0] != "none" {
		t.Errorf("prompt = %v, want [none]", got)
	}
	// The rest of the URL is the usual sign-in.
	if u.Query().Get("code_challenge_method") != "S256" || u.Query().Get("response_type") != "code" {
		t.Errorf("prompt changed the rest of the URL: %s", silent)
	}
	if empty, _, _ := c.StartSignIn(WithPrompt("")); strings.Contains(empty, "prompt=") {
		t.Errorf("an empty prompt must send nothing: %s", empty)
	}
}

func oauthEndpointForTest() oauth2.Endpoint {
	return oauth2.Endpoint{AuthURL: testIssuer + "/oauth/authorize", TokenURL: testIssuer + "/oauth/token"}
}
