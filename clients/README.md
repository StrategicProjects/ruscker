# Client packages for Ruscker sign-in

Thin packages that let another system sign people in through a Ruscker
portal ("Sign in with …"), one per language. They wrap an established
OpenID Connect / JWT library — none of them implements cryptography —
and are pre-set for the provider's profile, so an integration is a
handful of lines instead of a reading of the specification.

| Directory | Language | For |
|---|---|---|
| [`js/`](js/) | TypeScript | the browser half (start the sign-in, read the return) and a Node backend |
| [`python/`](python/) | Python | a backend (FastAPI, Flask, Django, anything) |
| [`rust/`](rust/) | Rust | a backend (axum, or anything with `reqwest`) |
| [`go/`](go/) | Go | a backend (`net/http`) |
| [`r/`](r/) | R | Shiny and plumber2 apps outside the portal, and reading the identity headers inside it |

The provider itself is described in the
[authentication guide](../book/src/authentication.md).

## The contract every package follows

The names change with each language's conventions; the behaviour does
not.

### Configuration

| Setting | Meaning |
|---|---|
| `issuer` | the portal's address, e.g. `https://portal.example.org` — exactly as its discovery document states it |
| `client_id` | the system's id, as registered on the portal |
| `client_secret` | the secret shown once at registration. Backend only — never in a browser |
| `redirect_uri` | the system's callback, exactly as registered |
| `scopes` | default `openid profile email groups roles setor` |

Endpoints are read from `{issuer}/.well-known/openid-configuration`,
once, and cached. The `issuer` in that document must equal the
configured one.

### Starting a sign-in

Produces the authorization URL plus the three values to keep until the
person comes back — `state`, `nonce` and the PKCE `code_verifier` —
each from the platform's cryptographic random source (at least 32 bytes
of entropy). Always: `response_type=code`, `code_challenge_method=S256`.
The provider accepts nothing else.

Where the three values are kept is the application's choice (a
server-side session, or a short-lived `HttpOnly` cookie); the package
hands them over as one value that can be stored and given back.

### Finishing a sign-in

Given the callback's query and the stored values:

1. An `error` parameter is reported as a failed sign-in, with its code.
2. `state` must equal the stored one (constant-time comparison where
   the language offers it).
3. If `iss` is present it must equal the issuer.
4. The code is exchanged at the token endpoint with the client secret
   (HTTP Basic), the `redirect_uri` and the `code_verifier`.
5. The ID token is verified: signature against the provider's JWKS,
   algorithm **ES256 only**, `iss`, `aud` = `client_id`, `exp`, and
   `nonce` = the stored one. A key id not in the cached JWKS triggers
   one refresh (the provider can rotate keys).
6. The result is the **identity**.

A code can be redeemed once; a failed exchange is never retried with
the same code.

The `error` of step 1 arrives in a callback nobody has verified yet
(anyone can craft such a link). Use its code to choose the
application's own message; do not show its text to the person as is.

### The identity

| Field | Claim | Notes |
|---|---|---|
| `sub` | `sub` | the person's permanent id on the portal. **The key to store.** A username can change hands; this cannot |
| `username` | `preferred_username` | |
| `name` | `name` | full name, or the username when the portal has none |
| `email` | `email` | may be absent |
| `setor` | `setor` | department; may be absent |
| `groups` | `groups` | list; only the groups that give access to this system when it is restricted |
| `group_roles` | `group_roles` | map of group → list of roles; `{}` when none |

Helpers: `has_group(g)`, `has_role(group, role)`, and `roles(group)`.
The raw claims stay available.

### What the packages do not do

* Sessions. After a sign-in the application issues its own session, as
  it does today; the portal's tokens live five minutes and there is no
  refresh.
* Sign-out at the portal. Ending the application's session is local.
* Deciding permissions. They report groups and roles; what those mean
  is the application's business.

## Status

Not published to any registry yet: install from this directory (each
package's README says how). The versions are `0.1.0`; the contract
above is what stays stable.

## Testing against a real provider

`testing/provider.sh` starts a throwaway portal with the provider on,
one person and one registered system; `testing/authorize.sh` plays that
person's browser. Each package's end-to-end test uses them:

```sh
cargo build                                    # the ruscker binary
clients/testing/provider.sh target/debug/ruscker /tmp/sso-dev &
source /tmp/sso-dev/env
clients/testing/authorize.sh "<authorization URL>"   # → the callback URL
```

The environment file gives `RUSCKER_ISSUER`, `RUSCKER_CLIENT_ID`,
`RUSCKER_CLIENT_SECRET` and `RUSCKER_REDIRECT_URI`. End-to-end tests
are skipped when `RUSCKER_ISSUER` is not set, so each package's unit
tests run anywhere.
