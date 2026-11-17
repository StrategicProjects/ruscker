# ruscker-sso (TypeScript)

Sign people in to your system through a Ruscker portal ("Sign in with
…"), over OpenID Connect. A thin wrapper, pre-set for the provider's
profile (authorization code + PKCE `S256`, ES256 ID tokens), in two
halves:

| Import | Runs in | Does |
|---|---|---|
| `ruscker-sso/browser` | the browser | starts the sign-in, reads the return. No secret, no dependencies (Web Crypto) |
| `ruscker-sso/server` | Node ≥ 20 | exchanges the code with the client secret and verifies the ID token ([`jose`](https://github.com/panva/jose)) |

The browser never exchanges the code: the portal's clients are
confidential and the token endpoint wants the secret. It hands the code
to **your own backend**.

## Install

Not published to a registry. From a checkout:

```sh
cd clients/js && npm install && npm run build
# in your app:
npm install /path/to/clients/js
```

## Split flow (a frontend with its own backend) — the usual path

**Browser** — a "Sign in" page and the callback page
(`https://app.example.org/callback`, registered on the portal):

```js
import { redirectToSignIn, readCallback, signInButton, SignInError } from "ruscker-sso/browser";

const options = {
  issuer: "https://portal.example.org",
  clientId: "my-system",
  redirectUri: "https://app.example.org/callback",
};

// Sign-in page: keeps state/nonce/verifier in sessionStorage, goes to the portal.
document.body.append(
  signInButton({ label: "Sign in with the portal", onClick: () => redirectToSignIn(options) }),
);

// Callback page: checks state and iss, then posts to YOUR backend.
try {
  const { code, codeVerifier, nonce } = readCallback();
  history.replaceState(null, "", location.pathname); // drop the code from the address bar
  const res = await fetch("/api/auth/exchange", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ code, codeVerifier, nonce }),
  });
  if (!res.ok) throw new Error("sign-in failed");
  location.assign("/");
} catch (e) {
  // ProviderError (e.error === "access_denied" …), StateMismatchError, IssuerMismatchError, NoPendingSignInError
  showMessage(e instanceof SignInError ? e.message : "sign-in failed");
}
```

In React, call `redirectToSignIn` from a button's `onClick` and
`readCallback` once in the callback route's effect.

**Backend** (Express, with `express-session`):

```js
import express from "express";
import session from "express-session";
import { SignInClient, SignInError } from "ruscker-sso/server";

const sso = new SignInClient({
  issuer: "https://portal.example.org",
  clientId: "my-system",
  clientSecret: process.env.SSO_CLIENT_SECRET,
  redirectUri: "https://app.example.org/callback",
});

const app = express();
app.use(express.json(), session({ secret: process.env.SESSION_SECRET, resave: false, saveUninitialized: false }));

app.post("/api/auth/exchange", async (req, res) => {
  try {
    const who = await sso.exchangeCode(req.body); // { code, codeVerifier, nonce }
    req.session.user = { sub: who.sub, name: who.name, groups: who.groups, groupRoles: who.groupRoles };
    res.sendStatus(204);
  } catch (e) {
    if (!(e instanceof SignInError)) throw e;
    res.status(401).json({ error: e.name });
  }
});
```

## All-backend flow

The backend does everything; the pending values live in its session.

```js
app.get("/login", async (req, res) => {
  const { url, pending } = await sso.startSignIn();
  req.session.pending = pending;
  res.redirect(url);
});

app.get("/callback", async (req, res) => {
  const pending = req.session.pending;
  delete req.session.pending; // used once, whatever happens
  try {
    if (!pending) throw new Error("no sign-in in progress");
    const who = await sso.finishSignIn(req.query, pending);
    req.session.regenerate(() => {
      req.session.user = { sub: who.sub, name: who.name, groups: who.groups };
      res.redirect("/");
    });
  } catch (e) {
    res.status(401).send(`Sign-in failed: ${e.message}`);
  }
});
```

`finishSignIn` runs, in order: the provider's `error`, `state`
(constant-time), `iss`, the code exchange (HTTP Basic + PKCE verifier),
the ID token (JWKS signature, **ES256 only**, `iss`, `aud`, `exp`,
`nonce`). The first three need no network call.

## The identity

| Field | Claim | Notes |
|---|---|---|
| `sub` | `sub` | permanent id. **Store this**, not the username or e-mail |
| `username` | `preferred_username` | |
| `name` | `name` | full name, or the username |
| `email` | `email` | may be `undefined` |
| `setor` | `setor` | department; may be `undefined` |
| `groups` | `groups` | only the groups that give access to this system |
| `groupRoles` | `group_roles` | `{ group: [roles] }`, `{}` when none |
| `claims` | — | every raw claim |

Helpers: `hasGroup(g)`, `hasRole(group, role)`, `roles(group)`.

The portal's tokens last five minutes and there is no refresh: after the
sign-in your application keeps its own session. What groups and roles
allow is your application's decision.

## Errors

All extend `SignInError`.

| Class | When |
|---|---|
| `ProviderError` | the portal sent `?error=…` (`.error`, e.g. `access_denied`) |
| `StateMismatchError` | `state` missing or different — not this browser's sign-in |
| `IssuerMismatchError` | `iss` (callback or discovery) is not the configured issuer |
| `NoPendingSignInError` | browser: nothing stored (direct visit, other tab, already used) |
| `TokenExchangeError` | the token endpoint refused (`.status`, `.error`: `invalid_client`, `invalid_grant`). Never retry the code |
| `IdTokenError` | the ID token failed a check (`.reason`) |
| `DiscoveryError` | the discovery document could not be read |

## Tests

```sh
npm install
npm test                 # unit tests, no network
# end to end, against clients/testing/provider.sh:
source /path/to/workdir/env && npm run test:e2e
npm run test:all         # both
```

The end-to-end tests are skipped when `RUSCKER_ISSUER` is not set.
