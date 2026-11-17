# Authentication

Ruscker keeps **one account base** — people, their groups and their
two-step verification — and uses it in three places:

1. **the portal and the admin panel** — who may see which apps and who
   may manage what;
2. **apps hosted behind the proxy** — which receive the signed-in user's
   identity in request headers;
3. **systems that live somewhere else** — other hosts, other domains —
   which can sign users in through Ruscker with OpenID Connect, the
   protocol behind "Sign in with Google".

![One account base in Ruscker (accounts, groups, 2FA). The user signs in once. A hosted app behind /app/{id} receives identity headers, per request, added by the proxy. An external system on another host or domain receives an ID token through OpenID Connect, once per sign-in, verified by the system.](images/auth-overview.svg)

This page is the map. The day-to-day screens are described in
[The admin panel](./admin.md); the YAML keys in
[Configuration](./configuration.md).

## Accounts, roles and groups

Everyone signs in at `/admin/login` with a username and password
(argon2id hashes; at least 8 characters with upper- and lower-case, a
digit and a symbol). An account created by an administrator must change
its password at the first sign-in.

| What | Meaning |
|---|---|
| **Role** — Admin, Editor or Viewer | What the person may do *in the admin panel*. A Viewer is a portal user with no panel access; an Editor manages apps, media and the users of their own groups; an Admin manages everything. |
| **Groups** | What the person may *reach*: an app or an external system restricted to a group is visible and usable only by its members. |
| **Group roles** | Optional labels a group defines for its members (`gestor`, `analista`, …), forwarded to hosted apps and to SSO clients that ask for them. They mean nothing to Ruscker itself — a system can use them to derive its own profiles. |

The `RUSCKER_ADMIN_TOKEN` is a **break-glass** login for the operator
(`/admin/login?token=1`). It is not a person: it has no account, so it
can never be used to sign in to an external system.

Sessions are opaque server-side ids in an `HttpOnly`, `SameSite=Strict`
cookie, valid for 24 hours; logging out, a password reset, a role change
or deleting the account ends them.

### Two-step verification

Users enrol an authenticator app once, under **Account → 2FA**, and get
ten one-time recovery codes. Whether a second step is *required* is
decided where the access happens:

- a hosted app sets **Require 2FA** (`require-mfa`), with a freshness
  window of up to 30 days;
- an external system's client sets **Require two-step verification** —
  there the proof must have been given in the same login session.

## Hosted apps: identity in headers

An app behind `/app/{id}` or `/api/{id}` never sees a password or a
token. When its spec opts in, the proxy adds the signed-in user's
identity to **every request** (and to the WebSocket handshake):

| Opt-in | Header | Value |
|---|---|---|
| `add-default-http-headers: true` | `X-SP-UserId` | username |
| | `X-SP-UserGroups` | comma-separated groups |
| `identity-claims: [email]` | `X-Ruscker-User-Email` | e-mail, if set |
| `identity-claims: [setor]` | `X-Ruscker-User-Setor` | department, if set |
| `identity-claims: [roles]` | `X-Ruscker-User-Roles` | `GROUP:ROLE` pairs, group percent-encoded |

Two rules make these headers trustworthy:

- the proxy **removes** any `X-SP-*` or `X-Ruscker-User-*` header that
  arrives from outside, so a value the app sees was put there by Ruscker;
- everything is **off by default** — importing a spec never starts
  disclosing identity by itself.

A missing header means an anonymous visitor (or headers not enabled).
An app restricted to certain groups receives only those groups' roles.
Because one container may serve many users, read identity from the
request, never from an environment variable.

## External systems: sign in with Ruscker

A system outside the portal cannot read Ruscker's cookie — it lives on
another origin. Instead it sends the user to Ruscker and receives a
signed statement of who they are: an OpenID Connect **ID token**.

**Ruscker authenticates; the system authorizes.** The token says who the
user is and, on request, which of the system's groups they belong to.
What that person may do inside the system remains the system's own
decision.

![Authorization-code flow with PKCE between the external system's server, the user's browser and Ruscker as OIDC provider. 1: the browser opens the system. 2: the system redirects the browser to Ruscker's /oauth/authorize with a PKCE challenge. 3: Ruscker signs the user in and checks the client's groups and MFA requirement. 4: Ruscker redirects back with a one-use code, which the browser delivers to the system. 5: the system's server calls POST /oauth/token directly with its secret, the code and the PKCE verifier. 6: Ruscker returns an ES256-signed ID token. The system keeps its own session; Ruscker authenticates, the system authorizes.](images/auth-flow.svg)

### Turning it on

The provider is **off by default**; a deployment that never enables it
exposes nothing new. In the service configuration:

```yaml
server:
  oidc-provider:
    enabled: true
    issuer: https://portal.example.org   # this portal's public base URL
```

- `issuer` is the portal's **public** address, including the subpath
  when it is mounted under one (`https://example.org/box`). It is written
  into every token and compared byte for byte by clients, so it is
  configured, never guessed from the request. HTTPS only (plain HTTP is
  accepted for `localhost` during development).
- `RUSCKER_MASTER_KEY` must be set: the signing key is generated on
  first use and stored encrypted in the database, so every node of an HA
  deployment shares it.
- If the provider is enabled but cannot work — no issuer, an issuer
  whose path is not the mount point, no master key — it stays off and
  says why at startup.

### Registering a system

Under **SSO → Clients** (Admin only), register each system:

| Field | Notes |
|---|---|
| **Client ID** | Public identifier, 3–64 lowercase letters, digits, `.`, `-`, `_`. Permanent. |
| **Name** | Shown to people on the sign-in page and in the admin. |
| **Redirect URIs** | Where Ruscker may send the user back. Matched **exactly** — no wildcards, no prefixes. HTTPS, or loopback HTTP for development. |
| **Allowed groups** | Only members of at least one of these groups can sign in to the system. Empty means any signed-in user. An Admin account gets no bypass. |
| **Require two-step verification** | The user must have confirmed a second factor in this login session. |
| **System logo** | A Media-library image. With the name, it is shown on the sign-in page — "You are signing in to …" — when a user arrives from this system. |
| **What the system may receive** | Which optional scopes it can be granted: `profile`, `email`, `groups`, `roles`, `setor`. A scope left unticked is never granted, even if the system asks for it; `openid` (the person's id) always is. Systems registered before this setting existed are unrestricted until edited. |

The **client secret** is shown once, when the client is created or when
you generate a new one; only a salted hash is stored. Generating a new
secret invalidates the old one immediately.

### What the system's developers need

Give them three things: the **issuer URL**, the **client ID** and the
**client secret**. Any standard OpenID Connect client library reads the
rest from `{issuer}/.well-known/openid-configuration`.

| Setting | Value |
|---|---|
| Flow | Authorization code with PKCE (`S256`) |
| Client type | Confidential — the code exchange happens on the system's **server** |
| Client authentication | `client_secret_basic` or `client_secret_post` |
| Scopes | `openid`, plus `profile`, `email`, `setor`, `groups`, `roles` as needed |
| ID token signature | `ES256`, keys at `{issuer}/oauth/jwks` |
| Token lifetime | 5 minutes (the system keeps its own session) |

Claims, by scope:

| Scope | Claims |
|---|---|
| `openid` | `sub` — the account's permanent id — plus `iss`, `aud`, `exp`, `iat`, and `nonce` when one was sent |
| `profile` | `preferred_username`; `name` — the person's full name when the account has one, the username otherwise |
| `email` | `email`, when the account has one |
| `setor` | `setor` (department), when the account has one |
| `groups` | `groups` — only the groups that grant access to the system when it is restricted, all of the user's groups when it is open |
| `roles` | `group_roles` — an object mapping each of those groups to the [group roles](./admin.md#groups) the user holds in it, e.g. `{"fiscal": ["analista", "gestor"]}`; `{}` when there are none. Its keys are group names, so this scope names those groups even without `groups` |

The same claims are available from `{issuer}/oauth/userinfo` with the
access token.

### Client packages

The repository carries small client packages — TypeScript, Python,
Rust, Go and R — that do everything in the next section on top of each
language's established OpenID Connect or JWT library:
[`clients/`](https://github.com/StrategicProjects/ruscker/tree/main/clients).
They share one contract (start a sign-in, finish it, read the identity)
and are tested end to end against a real portal. Using one is optional;
any standard library works with the settings above.

### What the system must verify

The provider cannot do this part for the client:

- **Validate the ID token**: the signature (keys from the JWKS), `iss`,
  `aud` equal to its own client ID, `exp`, and the `nonce` it sent. The
  `aud` check is what stops a token issued to another system from being
  replayed at this one.
- **Bind `state` and the PKCE verifier** to the user's browser session.
- **Identify the person by `sub`**, not by username or e-mail: a
  username can be reused after an account is deleted, and `email` is
  neither verified nor unique (there is no `email_verified` claim).
- **Never accept a bearer token received from elsewhere** by calling
  `/oauth/userinfo` with it: that answer says whose token it is, not
  which system it was issued to.
- **Keep its own session** after the exchange. Removing someone from the
  client's groups takes effect at that system's next sign-in.

A minimal check of the ID token in Python, with PyJWT:

```python
import jwt, json, urllib.request
from jwt import PyJWKClient

issuer, client_id = "https://portal.example.org", "my-system"
conf = json.load(urllib.request.urlopen(issuer + "/.well-known/openid-configuration"))
key = PyJWKClient(conf["jwks_uri"]).get_signing_key_from_jwt(id_token)
claims = jwt.decode(
    id_token, key.key,
    algorithms=["ES256"], audience=client_id, issuer=issuer,
    options={"require": ["exp", "iat", "sub", "aud", "iss"]},
)
assert claims["nonce"] == nonce_sent_with_the_request
person = claims["sub"]
```

### A static site, or a system without its own login

A site with no server of its own cannot hold a client secret. Put an
OpenID Connect-aware reverse proxy in front of it and register **that
proxy** as the client: it performs the sign-in and only then serves the
files. Two things to check in the proxy's own documentation:

- PKCE is often opt-in (for example `--code-challenge-method=S256` in
  oauth2-proxy, `OIDCPKCEMethod S256` in mod_auth_openidc);
- some proxies expect an `email` claim by default, so accounts without
  an e-mail fail there unless the proxy is told to use another claim.

### Where a sign-in can stop

| What the user or the system sees | Why |
|---|---|
| `404` on every provider URL | The provider is off, or `issuer` does not match the path the portal is mounted under. |
| `503` | `RUSCKER_MASTER_KEY` is not set. |
| A page saying *unknown client or redirect_uri* | The client ID is not registered, the client is disabled, or the redirect URI is not exactly one of those registered. Nothing is redirected in this case, on purpose. |
| Back at the system with `error=access_denied` | The user is not in the client's allowed groups, or signed in with the break-glass token. |
| Back at the system with `error=invalid_request` | The request lacks PKCE (`S256`), repeats a parameter, or asks for something outside the profile. |
| The password-change or 2FA page | A first sign-in with an administrator-assigned password, or a client that requires two-step verification. The sign-in continues afterwards. |
| `invalid_client` from the token endpoint | Wrong client secret, or the client is disabled. |
| `invalid_grant` from the token endpoint | The code was already used, expired (60 seconds), or is being redeemed with a different redirect URI or PKCE verifier; or the user lost access meanwhile. |

Each successful sign-in to an external system, and each refusal of a
person the portal could identify, is listed under **SSO → Access** (and in the general **Activity** page)
with the user, the system, the address and browser the user came from
and, for a refusal, the reason.

## API reference

For most integrations a standard OpenID Connect library needs only the
issuer URL and reads the rest from the discovery document. This section
is for calling the endpoints by hand, or for checking what a library is
doing.

All paths are relative to the **issuer** (`https://portal.example.org`
in the examples; include the subpath when the portal is mounted under
one). Every endpoint answers `404` while the provider is off and `503`
while `RUSCKER_MASTER_KEY` is not set.

| Method | Path | Called by | Authentication |
|---|---|---|---|
| `GET` | `/.well-known/openid-configuration` | anyone | none |
| `GET` | `/oauth/jwks` | anyone | none |
| `GET` | `/oauth/button.svg` | anyone | none |
| `GET` | `/oauth/authorize` | the user's browser | the user's Ruscker session |
| `POST` | `/oauth/token` | the system's server | client ID + secret |
| `GET`, `POST` | `/oauth/userinfo` | the system's server | access token |

### Discovery

```sh
curl https://portal.example.org/.well-known/openid-configuration
```

```json
{
  "issuer": "https://portal.example.org",
  "authorization_endpoint": "https://portal.example.org/oauth/authorize",
  "token_endpoint": "https://portal.example.org/oauth/token",
  "userinfo_endpoint": "https://portal.example.org/oauth/userinfo",
  "jwks_uri": "https://portal.example.org/oauth/jwks",
  "response_types_supported": ["code"],
  "response_modes_supported": ["query"],
  "grant_types_supported": ["authorization_code"],
  "subject_types_supported": ["public"],
  "id_token_signing_alg_values_supported": ["ES256"],
  "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
  "code_challenge_methods_supported": ["S256"],
  "authorization_response_iss_parameter_supported": true,
  "request_parameter_supported": false,
  "request_uri_parameter_supported": false,
  "scopes_supported": ["openid", "profile", "email", "groups", "roles", "setor"],
  "claims_supported": ["sub", "iss", "aud", "exp", "iat", "nonce",
                       "preferred_username", "name", "email", "setor",
                       "groups", "group_roles"]
}
```

Cacheable for five minutes (`Cache-Control: public, max-age=300`).

### Sign-in button — `GET /oauth/button.svg`

An SVG image reading "Sign in with *portal title*", for a system to put
on its own sign-in screen. Self-contained (no external font or image).

| Parameter | Values | Default |
|---|---|---|
| `theme` | `light`, `dark` | `light` |
| `lang` | `pt`, `en`, `es`, `fr` | `pt` |

```html
<a href="/login">
  <img src="https://portal.example.org/oauth/button.svg" height="40"
       alt="Sign in with the portal">
</a>
```

The link goes to the system's **own** sign-in route, which starts the
flow; the image is only a picture.

### Keys — `GET /oauth/jwks`

```sh
curl https://portal.example.org/oauth/jwks
```

```json
{
  "keys": [
    {
      "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
      "kid": "BgMPTK9aUYp5HAtXLX2tNsYpoUAC43ofCtC2ySEpZDQ",
      "x": "…", "y": "…"
    }
  ]
}
```

The `kid` in a token's header names the key that signed it. Also
cacheable for five minutes; a library that meets an unknown `kid` should
fetch the set again.

### Authorization — `GET /oauth/authorize`

The system redirects the **user's browser** here. Before that it
generates, and keeps in the user's session, a random `state`, a random
`nonce` and a PKCE pair:

```sh
verifier=$(openssl rand -hex 32)
challenge=$(printf '%s' "$verifier" | openssl dgst -sha256 -binary \
            | openssl base64 -A | tr '+/' '-_' | tr -d '=')
```

| Parameter | Required | Value |
|---|---|---|
| `response_type` | yes | `code` |
| `client_id` | yes | the registered client ID |
| `redirect_uri` | yes | exactly one of the registered URIs |
| `scope` | yes | space-separated; must contain `openid`. Also understood: `profile`, `email`, `setor`, `groups`, `roles`. Anything else is ignored, and so is a scope the system is not allowed (see **What the system may receive**): the `scope` in the token response says what was granted. |
| `code_challenge` | yes | the PKCE challenge (43 characters) |
| `code_challenge_method` | yes | `S256` |
| `state` | recommended | opaque value returned unchanged (up to 1024 characters) |
| `nonce` | recommended | returned inside the ID token (up to 255 printable ASCII characters) |
| `prompt` | no | `none` to get an error instead of a sign-in page when the user would have to interact |
| `response_mode` | no | only `query` |

```text
https://portal.example.org/oauth/authorize?response_type=code
    &client_id=my-system
    &redirect_uri=https%3A%2F%2Fmy-system.example.org%2Fauth%2Fcallback
    &scope=openid+profile+groups
    &state=af0ifjsldkj&nonce=n-0S6_WzA2Mj
    &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
    &code_challenge_method=S256
```

**On success** the browser is redirected (`302`) to the redirect URI:

```text
https://my-system.example.org/auth/callback?code=3f9c…&iss=https%3A%2F%2Fportal.example.org&state=af0ifjsldkj
```

The `code` is valid for 60 seconds and for one use. Check that `state`
is the one you sent and that `iss` is your issuer.

If the user is not signed in, Ruscker shows its sign-in page first (and
the password-change or two-step page when one applies) and then
continues to the redirect above.

**On failure** there are two cases:

- The client ID or the redirect URI did not check out, a parameter is
  repeated, or the request is malformed (more than 32 parameters, a
  query longer than 4096 bytes, a backslash). The user sees a `400`
  page; **nothing is redirected**.
- Anything else goes back to the redirect URI as
  `?error=…&iss=…&state=…`:

| `error` | Meaning |
|---|---|
| `invalid_request` | PKCE missing or not `S256`, a malformed `nonce`, an unsupported `response_mode`, `prompt=none` combined with another value |
| `unsupported_response_type` | `response_type` is not `code` |
| `invalid_scope` | `scope` lacks `openid` |
| `request_not_supported`, `request_uri_not_supported` | request objects are not supported |
| `access_denied` | the user is not in the client's allowed groups, or has no account (break-glass token session) |
| `login_required`, `interaction_required` | with `prompt=none`: the user would have had to sign in, change a password or confirm a second factor |
| `server_error` | Ruscker could not complete the request |

### Token — `POST /oauth/token`

Called by the system's **server** with the code from the callback. The
body is `application/x-www-form-urlencoded`, at most 4 KiB.

| Parameter | Required | Value |
|---|---|---|
| `grant_type` | yes | `authorization_code` |
| `code` | yes | the code from the callback |
| `redirect_uri` | yes | the same redirect URI used in the authorization request |
| `code_verifier` | yes | the PKCE verifier (43–128 characters of `A-Z a-z 0-9 - . _ ~`) |

The client authenticates in **one** of two ways — never both:

```sh
# HTTP Basic (client_secret_basic)
curl -u "my-system:$CLIENT_SECRET" https://portal.example.org/oauth/token \
  -d grant_type=authorization_code \
  -d code="$CODE" \
  --data-urlencode redirect_uri=https://my-system.example.org/auth/callback \
  -d code_verifier="$verifier"

# or in the body (client_secret_post)
curl https://portal.example.org/oauth/token \
  -d client_id=my-system -d client_secret="$CLIENT_SECRET" \
  -d grant_type=authorization_code -d code="$CODE" \
  --data-urlencode redirect_uri=https://my-system.example.org/auth/callback \
  -d code_verifier="$verifier"
```

```json
{
  "access_token": "eyJhbGciOiJFUzI1NiIsInR5cCI6ImF0K2p3dCIs…",
  "token_type": "Bearer",
  "expires_in": 300,
  "id_token": "eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIs…",
  "scope": "openid profile groups"
}
```

Responses carry `Cache-Control: no-store`.

The **ID token** is a JWT signed with ES256. Its header and payload:

```json
{ "alg": "ES256", "typ": "JWT", "kid": "BgMPTK9aUYp5HAtXLX2tNsYpoUAC43ofCtC2ySEpZDQ" }
```

```json
{
  "iss": "https://portal.example.org",
  "aud": "my-system",
  "sub": "5966f47e-a879-4aee-8588-69e44be72525",
  "iat": 1795042800,
  "exp": 1795043100,
  "nonce": "n-0S6_WzA2Mj",
  "preferred_username": "ana",
  "name": "ana",
  "groups": ["fiscal"]
}
```

`preferred_username` and `name` need the `profile` scope, `email` the
`email` scope (and an e-mail on the account), `setor` the `setor` scope,
`groups` the `groups` scope and `group_roles` the `roles` scope. The
`scope` returned lists what was granted, in the provider's own order.
There is no `auth_time`.

Errors are JSON, `{"error": "…", "error_description": "…"}`:

| Status | `error` | Meaning |
|---|---|---|
| `401` | `invalid_client` | unknown or disabled client, wrong secret, or an `Authorization` header that is not readable Basic. With Basic, the response carries `WWW-Authenticate: Basic`. |
| `400` | `invalid_request` | wrong content type, a repeated or missing parameter, both authentication methods at once |
| `400` | `unsupported_grant_type` | `grant_type` is not `authorization_code` |
| `400` | `invalid_grant` | the code is unknown, already used or expired; or it is being redeemed by another client, with another redirect URI or with a verifier that does not match; or the user no longer has access |
| `413` | — | the body is larger than 4 KiB |
| `500`, `503` | `server_error` | Ruscker could not complete the request (for example the signing key is unavailable) |

A failed **client authentication** does not spend the code. Once the
client is authenticated, presenting the code spends it whatever the
outcome — there is no second attempt with the same code.

### UserInfo — `GET` or `POST /oauth/userinfo`

```sh
curl -H "Authorization: Bearer $ACCESS_TOKEN" https://portal.example.org/oauth/userinfo
```

```json
{
  "sub": "5966f47e-a879-4aee-8588-69e44be72525",
  "preferred_username": "ana",
  "name": "ana",
  "groups": ["fiscal"]
}
```

The same identity claims as the ID token, for the scopes that were
granted, read from the database at the time of the call. The access
token is valid for five minutes and only here. A missing, expired or
foreign token — and an account or client that has since been removed,
disabled or lost access — answers `401` with
`WWW-Authenticate: Bearer error="invalid_token"`.

### What it deliberately does not do

The profile is narrow on purpose — fewer options, fewer ways to
misconfigure:

- no implicit or password flows, no refresh tokens, no dynamic client
  registration, no consent screen (every client is registered by an
  Admin);
- no single logout: signing out of Ruscker does not sign the user out of
  systems that already have their own session;
- `max_age`, `prompt=login` and `auth_time` are not supported;
- no self-service sign-up or password reset by e-mail.

Signing in **to** Ruscker with an external identity provider (Google,
Keycloak, …) is a separate, not yet built feature; see the
[roadmap](./roadmap.md).

### One more thing if you host other people's apps

Apps under `/app/…` share the portal's origin, and therefore the origin
of the sign-in page. That page is now worth tokens for other systems
too. If the portal hosts code you do not control, read the same-origin
notes in [Security](./security.md) before enabling the provider.
