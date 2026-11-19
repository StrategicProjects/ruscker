# ruscker-sso (Python)

Sign people in to a Python backend through a Ruscker portal ("Sign in
with …"). A thin wrapper over [PyJWT](https://pyjwt.readthedocs.io/)
and [httpx](https://www.python-httpx.org/) pre-set for the provider's
profile: authorization code + PKCE, ID tokens signed with ES256 only.
It follows the [contract shared by every client package](https://github.com/StrategicProjects/ruscker/blob/main/clients/README.md).

Two calls do the work: `start_sign_in()` builds the URL to send the
browser to, and `finish_sign_in()` checks the callback, redeems the code
and verifies the ID token, in that order. The package does not keep
sessions: after a sign-in, the application issues its own.

## Install

Python 3.10 or later.

```sh
pip install ruscker-sso          # or: uv add ruscker-sso
```

## Example: FastAPI

The three pending values live in Starlette's signed session cookie for
the few minutes between the two routes (`pip install fastapi uvicorn
itsdangerous`).

```python
import os

from fastapi import FastAPI, Request
from fastapi.responses import RedirectResponse
from starlette.middleware.sessions import SessionMiddleware

from ruscker_sso import PendingSignIn, RusckerSSO, SignInError

sso = RusckerSSO(
    issuer="https://portal.example.org",
    client_id="my-system",
    client_secret=os.environ["RUSCKER_CLIENT_SECRET"],
    redirect_uri="https://app.example.org/auth/callback",
)

app = FastAPI()
# Signed (not encrypted) cookie, HttpOnly; https_only in production.
app.add_middleware(SessionMiddleware, secret_key=os.environ["APP_SESSION_KEY"],
                   https_only=True, same_site="lax")


@app.get("/auth/login")
def login(request: Request):
    sign_in = sso.start_sign_in()
    request.session["ruscker_pending"] = sign_in.pending.to_json()
    return RedirectResponse(sign_in.url)


@app.get("/auth/callback")
def callback(request: Request):
    stored = request.session.pop("ruscker_pending", None)   # single use
    if stored is None:
        return RedirectResponse("/auth/login")
    try:
        person = sso.finish_sign_in(request.query_params, PendingSignIn.from_json(stored))
    except SignInError as exc:
        return RedirectResponse(f"/?signin_failed={type(exc).__name__}")
    # The application's own session from here on. Key the account by `sub`.
    request.session["user"] = {"sub": person.sub, "name": person.name,
                               "is_manager": person.has_role("fiscal", "gestor")}
    return RedirectResponse("/")
```

The calls are synchronous (they make two short HTTP requests on the
first sign-in, then one); FastAPI runs a plain `def` route in a thread
pool. The same two calls work in Flask (`session`, `request.args`) and
Django (`request.session`, `request.GET`): store `pending.to_json()`,
then pass the query and `PendingSignIn.from_json(...)` back.

Create one `RusckerSSO` at start-up and share it: it caches the
discovery document and the keys, and is thread-safe. Pass
`http_client=httpx.Client(...)` for proxies or a custom CA bundle.

## Signing out, the password page, silent checks

Since 0.2.0.

`sso.sign_out_url(*, post_logout_redirect_uri=None, state=None,
id_token_hint=None)` builds the address of the portal's sign-out page.
End the application's **own** session first, then redirect there: the
portal asks the person before ending its session and sends the browser
back to `post_logout_redirect_uri` (with `state`, at most 512
characters). That address must be one of the client's *Return addresses
after sign-out* on the portal, exactly. The endpoint comes from the
cached discovery document; a portal that does not announce one is older
than this feature and the call raises `SignInError`.

```python
@app.get("/auth/logout")
def logout(request: Request):
    request.session.clear()                          # the app's own session first
    return RedirectResponse(sso.sign_out_url(
        post_logout_redirect_uri="https://app.example.org/signed-out"))
```

`sso.password_change_url()` returns
`{issuer}/admin/account/password?client_id=…`, with no network call. The
portal signs the person in if needed and its page links back to the
client's *System address*:

```python
@app.get("/account/password")
def change_password():
    return RedirectResponse(sso.password_change_url())
```

`sso.start_sign_in(prompt="none")` asks the portal not to show a sign-in
page: a person still signed in there comes straight back with a code, and
anyone else comes back with `error=login_required` (`ProviderError`).
Use it when the application's own session expires, to notice that the
portal session ended.

## The identity

| Attribute | Claim | Notes |
|---|---|---|
| `sub` | `sub` | permanent id. **The key to store** |
| `username` | `preferred_username` | `None` without the `profile` scope |
| `name` | `name` | full name, or the username |
| `email` | `email` | may be `None` |
| `setor` | `setor` | department; may be `None` |
| `groups` | `groups` | list; only the groups that give access to this system when it is restricted |
| `group_roles` | `group_roles` | dict group → list of roles; `{}` when none |
| `claims` | — | every claim of the verified token |

Helpers: `has_group(g)`, `has_role(group, role)`, `roles(group)`.

Default scopes: `openid profile email groups roles setor`; pass
`scopes=[...]` to ask for less (`openid` is always added).

## Errors

All are `SignInError`; catch that one for "sign-in failed".

| Exception | When | Useful attributes |
|---|---|---|
| `ProviderError` | the callback carries `error=` (e.g. `access_denied`: not in the system's groups) | `code`, `description` |
| `StateMismatchError` | `state` differs from the stored one; nothing was sent to the provider | |
| `IssuerMismatchError` | the callback's `iss`, or the discovery document's `issuer`, is not the configured issuer | `expected`, `got`, `where` |
| `TokenExchangeError` | the token endpoint refused the code (`invalid_grant`: used, expired, other verifier; `invalid_client`: wrong secret) | `status`, `error`, `description` |
| `InvalidIDTokenError` | the ID token failed verification | `reason`: `signature`, `algorithm`, `audience`, `issuer`, `expired`, `not_yet_valid`, `nonce`, `missing_claim`, `unknown_key`, `malformed`, `claims` |
| `SignInError` itself | provider unreachable, malformed response, no `code`, unreadable stored values | |

A code works once. Whatever the outcome, start a new sign-in rather
than retrying.

## Tests

```sh
cd clients/python
uv run --with pytest --with-editable . pytest            # unit tests, no network
```

The end-to-end test runs against a real provider started with
`clients/testing/provider.sh` (see [the testing notes](https://github.com/StrategicProjects/ruscker/blob/main/clients/README.md#testing-against-a-real-provider))
and is skipped when `RUSCKER_ISSUER` is unset:

```sh
source /tmp/sso-dev/env
uv run --with pytest --with-editable . pytest -v tests/test_e2e.py
```
