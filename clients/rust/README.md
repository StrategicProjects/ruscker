# ruscker-sso (Rust)

A thin client that signs people in to a Rust backend through a Ruscker
portal ("Sign in with …"). The portal is an OpenID Connect provider with a
narrow profile — authorization code + PKCE `S256`, a confidential client,
ES256 ID tokens — and this crate is pre-set for exactly that. It follows
the [shared contract](../README.md).

It implements no cryptography: ID tokens are verified by
[`jsonwebtoken`](https://crates.io/crates/jsonwebtoken) against the
provider's JWKS (ES256 only, `iss`, `aud`, `exp`, `nonce`), random values
come from the OS (`getrandom`), the PKCE hash from `sha2`. HTTP is
`reqwest` with rustls; both use the **aws-lc-rs** crypto backend, so the
crate carries one crypto library (it needs a C compiler to build, as any
`reqwest` 0.13 with rustls does).

MSRV: Rust 1.85.

## Install

Not published to a registry; depend on it by path or git:

```toml
[dependencies]
ruscker-sso = { path = "clients/rust" }
```

## Example (axum 0.8)

The three values kept between the two routes go in a **private
(encrypted) cookie**, here with `axum-extra`'s `PrivateCookieJar`
(`features = ["cookie-private"]`). After the callback the application
issues its **own** session — the portal's tokens live five minutes and are
not refreshed.

```rust
use axum::{extract::{FromRef, RawQuery, State}, response::{IntoResponse, Redirect}, routing::get, Router};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar, SameSite};
use ruscker_sso::{Client, Config, Error, Identity};

#[derive(Clone)]
struct App { sso: Client, key: Key }
impl FromRef<App> for Key { fn from_ref(a: &App) -> Key { a.key.clone() } }

const PENDING: &str = "sso_pending";

async fn sign_in(State(app): State<App>, jar: PrivateCookieJar) -> impl IntoResponse {
    let (url, pending) = app.sso.start_sign_in();
    let cookie = Cookie::build((PENDING, serde_json::to_string(&pending).unwrap()))
        .http_only(true).secure(true).same_site(SameSite::Lax) // Lax: it must come back on the redirect
        .path("/auth").max_age(time::Duration::minutes(10));
    (jar.add(cookie), Redirect::to(url.as_str()))
}

async fn callback(State(app): State<App>, jar: PrivateCookieJar, RawQuery(q): RawQuery) -> impl IntoResponse {
    // Read and drop the pending values: they are single-use whatever happens.
    let pending = jar.get(PENDING).and_then(|c| serde_json::from_str(c.value()).ok());
    let jar = jar.remove(Cookie::build(PENDING).path("/auth"));
    let Some(pending) = pending else { return (jar, Redirect::to("/auth/sign-in")).into_response() };

    match app.sso.finish_sign_in(q.as_deref().unwrap_or(""), &pending).await {
        Ok(identity) => {
            let session = start_app_session(&identity); // your own session, keyed by identity.sub
            (jar.add(Cookie::build(("session", session)).http_only(true).secure(true).path("/")), Redirect::to("/"))
                .into_response()
        }
        Err(Error::Provider { error, .. }) if error == "access_denied" => {
            (jar, "You do not have access to this system.").into_response()
        }
        Err(e) => {
            eprintln!("sign-in failed: {e}");
            (jar, Redirect::to("/auth/sign-in")).into_response()
        }
    }
}

fn start_app_session(identity: &Identity) -> String { /* store, return an opaque id */ identity.sub.clone() }

#[tokio::main]
async fn main() {
    let config = Config::new(
        "https://portal.example.org",
        "my-system",
        std::env::var("SSO_CLIENT_SECRET").unwrap(),
        "https://app.example.org/auth/callback",
    );
    let app = App { sso: Client::discover(config).await.unwrap(), key: Key::generate() /* load a fixed key in production */ };
    let router = Router::new()
        .route("/auth/sign-in", get(sign_in))
        .route("/auth/callback", get(callback))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
```

### Mapping groups and roles to the application's profiles

What a group or role means is the application's decision:

```rust
enum Profile { Manager, Analyst }

fn profile(id: &ruscker_sso::Identity) -> Option<Profile> {
    if id.has_role("fiscal", "gestor") { Some(Profile::Manager) }
    else if id.has_role("fiscal", "analista") { Some(Profile::Analyst) }
    else { None } // signed in, but no profile in this system
}
```

## API

| | |
|---|---|
| `Config::new(issuer, client_id, client_secret, redirect_uri)` | `.scopes("…")` to change the default `openid profile email groups roles setor` |
| `Client::discover(config).await` | reads `{issuer}/.well-known/openid-configuration` and the JWKS once; refuses a document whose `issuer` differs. `discover_with(config, reqwest::Client)` to bring your own HTTP client (disable redirects on it) |
| `client.start_sign_in()` | `(Url, Pending)` — redirect to the URL, keep the `Pending` (serde) bound to the browser |
| `client.finish_sign_in(query, &pending).await` | `Result<Identity, Error>`; accepts the query with or without `?`, or the full callback URL |
| `exchange_code`, `verify_id_token` | the two network steps of `finish_sign_in`, for tests and special cases |

`Client` is cheap to clone. The cached JWKS is refreshed once when a token
names an unknown key id.

### `Identity`

| Field | Claim | |
|---|---|---|
| `sub: String` | `sub` | the permanent id — **the key to store** (usernames can be reused) |
| `username: Option<String>` | `preferred_username` | with the `profile` scope |
| `name: Option<String>` | `name` | full name, or the username when there is none |
| `email: Option<String>` | `email` | may be absent; not verified, not unique |
| `setor: Option<String>` | `setor` | department; may be absent |
| `groups: Vec<String>` | `groups` | for a restricted system, only the groups that give access |
| `group_roles: BTreeMap<String, Vec<String>>` | `group_roles` | empty when none |
| `claims: serde_json::Map` | all | the raw verified claims |

Helpers: `has_group(g)`, `has_role(g, r)`, `roles(g) -> &[String]`.

### `Error`

| Variant | When |
|---|---|
| `Provider { error, description }` | the callback carried `error=…` (e.g. `access_denied`: not in the system's groups) |
| `StateMismatch` | `state` missing or different — stale tab, second sign-in, forgery; nothing was sent to the provider |
| `IssuerMismatch { expected, found }` | the callback's `iss`, or the discovery document, names another issuer |
| `InvalidCallback(reason)` | no `code`, or a repeated parameter |
| `TokenRefused { status, error, description }` | the token endpoint refused: `invalid_client` (wrong secret), `invalid_grant` (code used, expired, not ours) |
| `InvalidIdToken(reason)` | signature, algorithm, `iss`, `aud`, `exp` or `nonce` failed |
| `Discovery(reason)` | the discovery document or key set is unusable |
| `Http(reqwest::Error)` | the provider could not be reached or sent unexpected JSON |

A code is used once; the crate never retries an exchange.

## Tests

Use a target directory of your own if your machine sets a shared one.

```sh
cargo test                      # unit tests, no network; e2e tests skip themselves

# end-to-end, against a throwaway provider (from the repository root):
cargo build && clients/testing/provider.sh target/debug/ruscker /tmp/sso-dev &
source /tmp/sso-dev/env
cd clients/rust && cargo test   # now also runs tests/e2e.rs and the key-refresh test
```
