# rusckersso — Ruscker sign-in for Go

Lets a Go backend sign people in through a Ruscker portal ("Sign in
with …") using OpenID Connect. It is a thin wrapper over
[`coreos/go-oidc`](https://github.com/coreos/go-oidc) (discovery, keys,
ID-token signature) and [`golang.org/x/oauth2`](https://pkg.go.dev/golang.org/x/oauth2)
(code exchange, PKCE), pre-set for the provider's profile: authorization
code + PKCE `S256`, ES256 only, client secret sent in an HTTP Basic
header. It adds the checks those libraries leave to you — the callback's
`state` and `iss`, and the token's `nonce` — and returns the person's
identity. The contract it follows is in [`../README.md`](https://github.com/StrategicProjects/ruscker/blob/main/clients/README.md).

It does not manage sessions: after a sign-in, your application issues
its own, as it does today.

## Install

```sh
go get github.com/StrategicProjects/ruscker/clients/go
```

Requires Go 1.25 or later (the floor set by `go-oidc` v3.21).

## Example (`net/http`)

```go
package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"os"

	rusckersso "github.com/StrategicProjects/ruscker/clients/go"
)

const pendingCookie = "sso_pending"

func main() {
	// One client for the whole process: it reads the discovery document
	// once and caches the provider's keys.
	sso, err := rusckersso.New(context.Background(), rusckersso.Config{
		Issuer:       "https://portal.example.org",
		ClientID:     "my-system",
		ClientSecret: os.Getenv("SSO_CLIENT_SECRET"),
		RedirectURI:  "https://app.example.org/auth/callback",
	})
	if err != nil {
		log.Fatal(err)
	}

	// 1. Start: keep the pending values in a short-lived HttpOnly cookie
	//    and send the browser to the portal.
	http.HandleFunc("/auth/login", func(w http.ResponseWriter, r *http.Request) {
		authURL, pending, err := sso.StartSignIn()
		if err != nil {
			http.Error(w, "cannot start sign-in", http.StatusInternalServerError)
			return
		}
		b, _ := json.Marshal(pending)
		http.SetCookie(w, &http.Cookie{
			Name: pendingCookie, Value: base64.RawURLEncoding.EncodeToString(b),
			Path: "/auth", MaxAge: 600, HttpOnly: true, Secure: true,
			SameSite: http.SameSiteLaxMode, // Lax: the cookie must come back on the portal's redirect
		})
		http.Redirect(w, r, authURL, http.StatusFound)
	})

	// 2. Callback: finish the sign-in, then issue the app's own session.
	http.HandleFunc("/auth/callback", func(w http.ResponseWriter, r *http.Request) {
		var pending rusckersso.Pending
		if c, err := r.Cookie(pendingCookie); err == nil {
			if b, err := base64.RawURLEncoding.DecodeString(c.Value); err == nil {
				_ = json.Unmarshal(b, &pending) // a bad cookie leaves it empty → state mismatch
			}
		}
		// The pending values are single-use: drop the cookie either way.
		http.SetCookie(w, &http.Cookie{Name: pendingCookie, Path: "/auth", MaxAge: -1})

		id, err := sso.FinishSignIn(r.Context(), r.URL.Query(), pending)
		var pe *rusckersso.ProviderError
		switch {
		case errors.As(err, &pe) && pe.Code == "access_denied":
			http.Error(w, "you do not have access to this system", http.StatusForbidden)
			return
		case err != nil:
			log.Printf("sign-in failed: %v", err)
			http.Error(w, "sign-in failed, please try again", http.StatusBadRequest)
			return
		}
		// id.Sub is the key to store: it never changes hands.
		startAppSession(w, id.Sub, id.Name, id.HasRole("fiscal", "gestor"))
		http.Redirect(w, r, "/", http.StatusFound)
	})

	log.Fatal(http.ListenAndServe(":8080", nil))
}

// startAppSession stands for whatever session mechanism the app already has.
func startAppSession(w http.ResponseWriter, sub, name string, isManager bool) { /* … */ }
```

The cookie holds the `state`, `nonce` and PKCE verifier only until the
person comes back (minutes); a server-side session works just as well.
`Pending` has JSON tags, so it can be stored as one value either way.

### The same two handlers with gin

`gin` is not a dependency of this package; this is only how the
handlers look there.

```go
r.GET("/auth/login", func(c *gin.Context) {
	authURL, pending, err := sso.StartSignIn()
	if err != nil {
		c.AbortWithStatus(http.StatusInternalServerError)
		return
	}
	b, _ := json.Marshal(pending)
	c.SetSameSite(http.SameSiteLaxMode)
	c.SetCookie(pendingCookie, base64.RawURLEncoding.EncodeToString(b), 600, "/auth", "", true, true)
	c.Redirect(http.StatusFound, authURL)
})

r.GET("/auth/callback", func(c *gin.Context) {
	var pending rusckersso.Pending
	if v, err := c.Cookie(pendingCookie); err == nil {
		if b, err := base64.RawURLEncoding.DecodeString(v); err == nil {
			_ = json.Unmarshal(b, &pending)
		}
	}
	c.SetCookie(pendingCookie, "", -1, "/auth", "", true, true)

	id, err := sso.FinishSignIn(c.Request.Context(), c.Request.URL.Query(), pending)
	if err != nil {
		c.String(http.StatusBadRequest, "sign-in failed, please try again")
		return
	}
	// e.g. with gin-contrib/sessions:
	//   s := sessions.Default(c); s.Set("sub", id.Sub); s.Save()
	c.Redirect(http.StatusFound, "/")
})
```

## The identity

| Field | Claim | Notes |
|---|---|---|
| `Sub` | `sub` | permanent id — **the key to store** |
| `Username` | `preferred_username` | |
| `Name` | `name` | full name, or the username |
| `Email` | `email` | `""` when absent |
| `Setor` | `setor` | department; `""` when absent |
| `Groups` | `groups` | never nil |
| `GroupRoles` | `group_roles` | group → roles; never nil |
| `Claims` | all | the raw decoded claims |

Helpers: `HasGroup(g)`, `HasRole(group, role)`, `Roles(group)`.

## Errors

`FinishSignIn` checks, in order, and stops at the first failure:

| Check | Error (`errors.Is`) | Typed (`errors.As`) |
|---|---|---|
| callback has `error=…` | `ErrProvider` | `*ProviderError` — `.Code` (`access_denied`, …), `.Description` |
| `state` ≠ stored | `ErrStateMismatch` | — |
| `iss` absent or ≠ issuer | `ErrIssuerMismatch` | — |
| no `code` | `ErrMissingCode` | — |
| token endpoint refused / unreachable | `ErrTokenExchange` | `*TokenExchangeError` — `.Code` (`invalid_grant`, `invalid_client`), `.StatusCode` |
| ID token invalid (signature, ES256, `iss`, `aud`, `exp`, `nonce`) | `ErrIDTokenInvalid` | `*IDTokenError` — `.Reason`, `.Err` |

The first three happen before any network call. A code is single-use:
on any failure, start a new sign-in. `New` fails if the discovery
document cannot be read or names a different issuer.

## Tests

```sh
go test ./...                      # unit tests, no network; end-to-end tests skip

# end-to-end, against the test provider (see ../README.md)
source /tmp/sso-dev/env
go test -count=1 -v -run E2E ./...
```
