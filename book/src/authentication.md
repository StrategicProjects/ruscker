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
| **Group roles** | Optional labels a group defines for its members (`gestor`, `analista`, …), forwarded to apps that ask for them. They mean nothing to Ruscker itself. |

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

Under **Credentials → SSO clients** (Admin only), register each system:

| Field | Notes |
|---|---|
| **Client ID** | Public identifier, 3–64 lowercase letters, digits, `.`, `-`, `_`. Permanent. |
| **Name** | Shown to people on the sign-in page and in the admin. |
| **Redirect URIs** | Where Ruscker may send the user back. Matched **exactly** — no wildcards, no prefixes. HTTPS, or loopback HTTP for development. |
| **Allowed groups** | Only members of at least one of these groups can sign in to the system. Empty means any signed-in user. An Admin account gets no bypass. |
| **Require two-step verification** | The user must have confirmed a second factor in this login session. |

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
| Scopes | `openid`, plus `profile`, `email`, `groups` as needed |
| ID token signature | `ES256`, keys at `{issuer}/oauth/jwks` |
| Token lifetime | 5 minutes (the system keeps its own session) |

Claims, by scope:

| Scope | Claims |
|---|---|
| `openid` | `sub` — the account's permanent id — plus `iss`, `aud`, `exp`, `iat`, and `nonce` when one was sent |
| `profile` | `preferred_username`, `name` |
| `email` | `email`, when the account has one |
| `groups` | `groups` — only the groups that grant access to the system when it is restricted, all of the user's groups when it is open |

The same claims are available from `{issuer}/oauth/userinfo` with the
access token.

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

Each successful sign-in to an external system is listed under
**Activity** as "SSO sign-in".

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
