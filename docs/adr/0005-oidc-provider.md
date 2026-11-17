# ADR 0005 — Ruscker as an OpenID Connect provider ("Sign in with Ruscker")

Status: accepted — slices 1–4 are implemented: the client registry, the
signing key and metadata endpoints, the authorization endpoint, and
the token and userinfo endpoints. A registered client can complete a
sign-in. Off by default.

## Context

Ruscker already owns everything an identity provider needs for the
people who use the portal: accounts with argon2id hashes and a password
policy, groups, TOTP step-up, sessions (in memory or shared Postgres),
login rate limiting, an audit log and a per-user activity log.

Systems that live **outside** Ruscker — not hosted behind `/app/*`, on
other hosts and domains — each grew their own login: one delegates to a
public identity provider, one keeps its own user table and opaque
sessions, one is a static site with no login at all. Operators want a
single account base and a single place to grant and revoke access.

Cookie sharing cannot solve this: those systems live on other origins.
The interoperable answer is to let Ruscker act as an **OpenID Connect
provider**, the same protocol behind "Sign in with Google".

This is the mirror image of external authentication (Ruscker as an OIDC
*client* of someone else's provider). The two compose: with both,
Ruscker brokers an upstream identity to its clients.

## Decision

Ruscker gains an optional OIDC provider, **off by default**
(`server.oidc-provider.enabled`), with a deliberately narrow profile:

| Aspect | Choice |
|---|---|
| Flow | Authorization Code with PKCE (S256) only. No implicit, no password grant, no device flow. |
| Clients | Confidential, registered by an Admin. No dynamic registration, no public clients. |
| Redirect URIs | Exact string match. HTTPS, or HTTP on loopback for development. No wildcards. |
| Signing | ES256. The key is generated and used through `ring` (already a dependency), stored encrypted under `RUSCKER_MASTER_KEY` so HA nodes share it. |
| Tokens | A short-lived ID token plus `/oauth/userinfo`. Clients keep their **own** session after the exchange. |
| Claims | `sub`, `preferred_username`, `name`, `email`, `groups`. Group roles join when they exist. |
| Access | Per client: `access-groups` (empty = any signed-in user) and optional `require-mfa`. |
| Consent screen | None. Every client is first-party and admin-registered. |

**Ruscker authenticates; the client authorizes.** The token says who the
user is and which groups they belong to. What that user may do inside
the client stays in the client.

### Why clients keep their own session

It removes refresh tokens, token introspection and single logout from
the first version, and it matches how the candidate clients already
work: they exchange an upstream code once and mint their own session.
Revoking access in Ruscker therefore takes effect at the client's next
login, not instantly — an accepted limit of the first version.

### Access gate semantics

A client's gate mirrors a spec's `access-groups`, with one intentional
difference. When the Groups page deletes a group and that would leave a
restricted client with no groups, the client is **disabled** rather than
opened. For a hosted app an empty list means "public", which is a
visible, reversible state; for a login gate, silently widening "this
team" to "everyone with an account" is the wrong way to fail.

Groups are derived from membership, so a group can also disappear
without being deleted (its last member leaves). The client then keeps
the orphan name: nobody can sign in, which is safe, but members later
added to a group of the same name regain access. Specs behave the same
way; it is stated here because a login gate makes it matter more.

The registry stores a client's `enabled` flag but `fetch` returns
disabled clients too. The authorization and token endpoints must check
it themselves.

## Slices

1. **Client registry** — `oidc_clients` table, Admin-only UI under
   Credentials, secret shown once and stored as a salted hash, audit
   rows. The registry screen is always available to Admins; the config
   flag gates the protocol endpoints, which do not exist yet.
   *(implemented)*
2. Signing key, `/.well-known/openid-configuration`, JWKS, the config
   flag. The key is created on the first JWKS request, stored encrypted
   in `oidc_keys`, and identified by its RFC 7638 thumbprint. The
   issuer is configured, never read from the request, and its path must
   be the portal's mount point. Exactly one first key is created, even
   under concurrent first use. *(implemented)*

   Key rotation is not built. When it is: clients cache the JWKS, so a
   new key must be published for at least the cache lifetime before it
   signs.
3. `/oauth/authorize`: session check or login, the client's group gate
   and MFA requirement, one-use authorization codes bound to PKCE.
   *(implemented)* Decisions taken here:
   - Until the client and its exact redirect URI are verified, nothing
     is redirected; the user sees a 400. Later errors go back to the
     client as `error=…`.
   - A repeated query parameter is refused outright.
   - `redirect_uri` is always required, PKCE is always required, and
     only `S256`.
   - No bypass of the group gate for Admin accounts, and a break-glass
     token session (no account) can never obtain a code.
   - The gate and `must_change_password` are read from the database on
     every request, never from the session.
   - MFA for a client is session-bound: the proof must have been given
     in the same login session (the "0 days" setting of hosted apps).
   - `prompt=none` is honoured (errors instead of pages); `prompt=login`
     and `max_age` are ignored, as is anything else not listed here.
   - The response carries `iss` (RFC 9207).
   - The session cookie stays `SameSite=Strict`. A browser arriving
     from a client on another site therefore sends no cookie; such a
     request is answered with a tiny page that re-requests the same URL
     from our own origin, so an existing session is recognised instead
     of asking for the password on every sign-in. Browsers that do not
     send `Sec-Fetch-Site` skip this and go to the login page.
   - `auth_time` is not emitted: the session's creation time is not
     recorded, and the code's issue time is not when the user
     authenticated.
   - What the client must do for its part: bind `state` and the PKCE
     verifier to its own browser session, check `iss`, verify `nonce`.
   - Codes live 60 seconds, are stored as a SHA-256 and are claimed by
     one conditional `UPDATE`.
4. `/oauth/token` and `/oauth/userinfo`. *(implemented)* Decisions
   taken here:
   - The client authenticates first (HTTP Basic or form body, never
     both — an `Authorization` header that is not readable Basic fails
     outright rather than falling back to the body); only then is the
     code redeemed. The request body is capped at 4 KiB and 32
     parameters, since this is reachable without credentials. A failed client
     authentication does not spend the code.
   - After redemption the code is spent whatever happens; every failed
     binding (other client, different redirect URI, PKCE mismatch, user
     gone or no longer in the client's groups) is the same
     `invalid_grant`.
   - The user is re-read at redemption and matched by account id, not
     name: a deleted and recreated username is a different person.
   - `sub` is the account's immutable id.
   - `groups` carries only the groups that grant access to the client
     when it is restricted, all of the user's groups when it is open.
   - The access token is a signed JWT (`typ: at+jwt`, audience = the
     issuer) accepted only by `/oauth/userinfo`; the ID token has
     `typ: JWT` and audience = the client, so neither can stand in for
     the other. Both live five minutes. Userinfo re-reads the user and
     the client, so it stops answering when access is revoked.
   - Each successful exchange is recorded in the user activity log as
     `sso.login`, with the client id.
   - Later additions: `name` carries the account's full name when set;
     scope `setor` → `setor`; scope `roles` → `group_roles`, an object
     of group → roles, filtered like `groups`. A system can derive its
     own profiles from group roles instead of keeping a user table.
     There is no per-client scope allowlist: every registered client
     may ask for any supported scope, consistent with clients being
     first-party systems registered by an Admin. A failed claims
     lookup is `server_error`, never an empty claim.
5. Integration recipes and a first real client.

Later, separately decided: RP-initiated logout, refresh tokens,
upstream providers.

## Consequences and risks

- **This is the most security-sensitive surface in the project.** Each
  slice needs an adversarial review beyond the test gate: redirect-URI
  handling, code replay, PKCE downgrade, mix-up between clients.
- **Same-origin hosted apps raise the stakes.** Apps under `/app/*`
  share an origin with the login page. Once that login is worth tokens
  for other systems, a compromised hosted app has more to gain. Serving
  untrusted apps from a separate origin should land before the provider
  is enabled in a deployment that hosts third-party code.
- **One issuer.** The issuer URL is part of every token. A deployment
  picks one public base URL and keeps it; browsers and client backends
  must both reach it.
- **Scope pressure.** An identity provider attracts feature requests
  (self-service signup, password reset by e-mail, social login). Those
  are out of scope; the profile above is the contract.

## Alternatives considered

- **Forward-auth only** (`auth_request` against a Ruscker endpoint).
  Simple, but depends on a shared cookie, so it only works for systems
  on Ruscker's own origin.
- **Run a dedicated identity provider next to Ruscker.** Mature and
  complete, but it creates a second account base and a second service
  to operate — the cost this ADR exists to avoid.
- **A proprietary "validate this session" API.** Less code up front,
  but every client would need custom glue, and none of the standard
  libraries or reverse-proxy helpers would work with it.
