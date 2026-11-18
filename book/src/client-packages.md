# Client packages

Small packages that let another system sign people in through a Ruscker
portal — "Sign in with …" — one per language. Each wraps an established
OpenID Connect or JWT library (none implements cryptography) and is
pre-set for the portal's profile, so an integration is two calls
instead of a reading of the specification.

Using one is optional: any standard OpenID Connect library works with
the settings in [Authentication](./authentication.md#what-the-systems-developers-need).

| Language | Package | Built on | For |
|---|---|---|---|
| TypeScript | [`ruscker-sso`](https://github.com/StrategicProjects/ruscker/tree/main/clients/js) | `jose` | a browser frontend and a Node backend |
| Python | [`ruscker_sso`](https://github.com/StrategicProjects/ruscker/tree/main/clients/python) | PyJWT, httpx | a backend (FastAPI, Flask, Django) |
| Rust | [`ruscker-sso`](https://github.com/StrategicProjects/ruscker/tree/main/clients/rust) | reqwest, jsonwebtoken | a backend (axum) |
| Go | [`rusckersso`](https://github.com/StrategicProjects/ruscker/tree/main/clients/go) | go-oidc, x/oauth2 | a backend (`net/http`, gin) |
| R | [`rusckersso`](https://github.com/StrategicProjects/ruscker/tree/main/clients/r) | httr2, jose | Shiny and plumber2 apps, outside or inside the portal |

> The packages live in the repository and are **not published to npm,
> PyPI, crates.io or CRAN yet**; install them from the repository as
> shown below. Each package's README has a complete, runnable example
> for the most common framework of its language.

## What every package does

The names follow each language's conventions; the behaviour is the
same everywhere and is written down in the
[contract](https://github.com/StrategicProjects/ruscker/blob/main/clients/README.md).

**Configuration** — four values, from the registration of the system
on the portal (**SSO → Clients**):

| Setting | Meaning |
|---|---|
| issuer | the portal's address, exactly as its discovery document states it |
| client id | the system's id |
| client secret | shown once at registration; **backend only**, never in a browser |
| redirect URI | the system's callback, exactly as registered |

**Starting a sign-in** returns the address to send the browser to, and
three random values to keep until the person comes back (`state`,
`nonce`, the PKCE verifier) — in a server-side session or a short-lived
`HttpOnly` cookie.

**Finishing a sign-in** takes the callback and those values, and, in
this order: reports an `error` from the portal; checks `state`; checks
that `iss` is the portal; exchanges the code using the secret; verifies
the ID token (signature against the portal's keys, **ES256 only**,
issuer, audience, expiry with 60 seconds of clock tolerance, `nonce`);
and returns the **identity**.

| Identity field | Notes |
|---|---|
| `sub` | the person's permanent id on the portal — **the key to store**; a username can change hands |
| username, name | the full name when the portal has one, the username otherwise |
| email, setor | when the account has them |
| groups | only the groups that give access to this system when it is restricted |
| group roles | group → roles; helpers `has_group`, `has_role`, `roles` |

Which of these a system receives is decided on the portal, per client
(see [what the system may receive](./authentication.md#registering-a-system)).

**What the packages do not do:** keep sessions (the application issues
its own after a sign-in; the portal's tokens live five minutes and
there is no refresh), sign out at the portal, or decide permissions —
they report groups and roles; what those mean is the application's
business.

## TypeScript

Two halves in one package. The **browser** half has no secret and no
dependency: it starts the sign-in and reads the return. The **server**
half holds the secret and does the exchange. A frontend with its own
backend uses both — the usual case:

```js
// in the browser
import { redirectToSignIn, readCallback } from "ruscker-sso/browser";

redirectToSignIn({ issuer, clientId, redirectUri });   // on "Sign in"
const { code, codeVerifier, nonce } = readCallback();  // on the callback page
// …POST these three to your own backend
```

```js
// on the backend
import { SignInClient } from "ruscker-sso/server";

const sso = new SignInClient({ issuer, clientId, clientSecret, redirectUri });
const who = await sso.exchangeCode(req.body);   // { code, codeVerifier, nonce }
// who.sub, who.name, who.hasRole("fiscal", "gestor")
```

A backend that renders its own pages can do everything itself with
`sso.startSignIn()` and `sso.finishSignIn(query, pending)`.

Install from a checkout: `cd clients/js && npm install && npm run
build`, then `npm install /path/to/clients/js` in the application.
Node 20 or later.

## Python

```python
from ruscker_sso import PendingSignIn, RusckerSSO

sso = RusckerSSO(issuer=..., client_id=..., client_secret=..., redirect_uri=...)

sign_in = sso.start_sign_in()            # redirect to sign_in.url,
stored = sign_in.pending.to_json()       # keep this in the session

person = sso.finish_sign_in(str(request.url), PendingSignIn.from_json(stored))
# person.sub, person.name, person.has_role("fiscal", "gestor")
```

Install: `pip install "git+https://github.com/StrategicProjects/ruscker#subdirectory=clients/python"`
(or `pip install ./clients/python` from a checkout). Python 3.10 or
later. The README has the complete FastAPI example.

## Rust

```rust
use ruscker_sso::{Client, Config};

let sso = Client::discover(config).await?;           // once, at start-up

let (url, pending) = sso.start_sign_in();            // redirect to `url`, keep `pending`
let id = sso.finish_sign_in(callback_query, &pending).await?;
// id.sub, id.name, id.has_role("fiscal", "gestor")
```

Depend on it by path, from a checkout (or by git):

```toml
[dependencies]
ruscker-sso = { path = "clients/rust" }
```

The README has the complete axum example, including how to turn group
roles into an application's own profiles.

## Go

```go
import rusckersso "github.com/StrategicProjects/ruscker/clients/go"

sso, err := rusckersso.New(ctx, rusckersso.Config{ /* issuer, id, secret, redirect URI */ })

authURL, pending, err := sso.StartSignIn()                 // redirect, keep `pending`
id, err := sso.FinishSignIn(ctx, r.URL.Query(), pending)
// id.Sub, id.Name, id.HasRole("fiscal", "gestor")
```

Install: `go get github.com/StrategicProjects/ruscker/clients/go`. Go
1.25 or later. The README has the complete `net/http` example and the
same two handlers for gin.

## R

Two different jobs, depending on where the app runs.

**Inside the portal** (a hosted Shiny or plumber2 app) there is no
sign-in to do: the portal already knows the person and passes the
identity in request headers, when the app's spec opts in with
`add-default-http-headers: true`.

```r
id <- rusckersso::hosted_identity(session)            # Shiny, in server()
id <- rusckersso::hosted_identity_plumber(request)    # plumber2, in a handler
```

**Outside the portal**, the same contract as the other languages:

```r
client <- sso_client(issuer = ..., client_id = ..., client_secret = ..., redirect_uri = ...)

start <- sso_start(client)                    # send the browser to start$url, keep start$pending
id <- sso_finish(client, query, pending)      # on the way back
has_role(id, "fiscal", "gestor")
```

Install:

```r
remotes::install_github("StrategicProjects/ruscker", subdir = "clients/r")
```

The README has a complete Shiny app, including how to keep the pending
values tied to the browser that started the sign-in.

## Handling a failed sign-in

Every package raises a distinct error for each way a sign-in can stop:
the portal refused it (with its code, for example `access_denied` when
the person is not in the system's groups), the `state` or the issuer
did not match, the code exchange was refused, or the ID token was not
valid.

Use the error's **kind or code** to choose your own wording. Do not
show its text to the person as it is: part of it arrives in the
callback address, which anyone can craft.

A code works once. After a failure, start a new sign-in; never retry
with the same callback.

## A "Sign in with …" button

The portal serves a ready-made button image carrying its own title, in
a light and a dark version — see
[Sign-in button](./authentication.md#sign-in-button--get-oauthbuttonsvg).
The link around it points at the application's own sign-in route.

## Testing an integration

The repository has a small harness that starts a throwaway portal with
the provider on, one person and one registered system, and a script
that plays that person's browser. Each package's end-to-end tests use
it, and it is just as useful for trying a new integration without
touching a real portal:

```sh
cargo build
clients/testing/provider.sh target/debug/ruscker /tmp/sso-dev &
source /tmp/sso-dev/env    # RUSCKER_ISSUER, RUSCKER_CLIENT_ID, RUSCKER_CLIENT_SECRET, RUSCKER_REDIRECT_URI
```
