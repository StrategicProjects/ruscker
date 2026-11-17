# ADR 0005 — Ruscker as an OpenID Connect provider ("Sign in with Ruscker")

Status: proposed (slices 1 and 2 are implemented: the client registry,
the signing key and the public metadata endpoints; the sign-in
endpoints are not — nothing here can issue a token yet)

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
| Tokens | A short-lived ID token plus `/userinfo`. Clients keep their **own** session after the exchange. |
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
   issuer is configured, never read from the request. *(implemented)*
3. `/oauth/authorize`: session check or login, the client's group gate
   and MFA requirement, one-use authorization codes bound to PKCE.
4. `/oauth/token` and `/userinfo`.
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
