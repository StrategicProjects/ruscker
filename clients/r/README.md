# rusckersso

Know who is using your R web app, through a Ruscker portal. Two halves:

* **Inside the portal** (an app served under `/app/<id>/` or `/api/<id>/`):
  read the identity the portal's proxy adds to every request. No sign-in
  code, no network.
* **Outside the portal** (a Shiny or plumber2 app on its own host):
  sign people in with the portal's OpenID Connect provider — the
  authorization code flow with PKCE, an ES256-signed ID token verified with
  [jose](https://cran.r-project.org/package=jose), HTTP via
  [httr2](https://cran.r-project.org/package=httr2).

Both return the same **identity** object.

## Install

```r
# from a checkout of the repository
install.packages(c("httr2", "jose", "jsonlite", "openssl", "rlang"))
install.packages("clients/r", repos = NULL, type = "source")
```

## Inside the portal

The app's spec must opt in — nothing is sent by default:

```yaml
add-default-http-headers: true          # X-SP-UserId, X-SP-UserGroups
identity-claims: [email, setor, roles]  # optional extras
```

```r
# Shiny, in server():
id <- rusckersso::hosted_identity(session)

# plumber2, in a handler:
id <- rusckersso::hosted_identity_plumber(request)
```

`NULL` means an anonymous visitor (or headers not enabled). A value can be
trusted because the proxy removes these headers from any request that comes
from outside — provided the app is reachable only through the portal.
Read it per request, never from an environment variable: one container
serves many people. The headers carry no permanent id, so `sub` is `NULL`
here; key on `username`.

## Outside the portal: a Shiny app

Register the app on the portal (**SSO → Clients**) with the app's own URL as
redirect URI, then:

```r
library(shiny)
library(rusckersso)

client <- sso_client(
  issuer        = "https://portal.example.org",
  client_id     = "my-app",
  client_secret = Sys.getenv("MY_APP_CLIENT_SECRET"),
  redirect_uri  = "https://app.example.org/"
)

# Sign-ins in progress, kept on the server and keyed by `state`.
pending_store <- new.env()

ui <- fluidPage(uiOutput("body"))

server <- function(input, output, session) {
  # Once, when the page has loaded (clientData is filled in by then).
  observeEvent(session$clientData$url_search, once = TRUE, {
    query <- parseQueryString(session$clientData$url_search)

    if (is.null(query$state)) {
      # No callback: offer to sign in.
      start <- sso_start(client)
      assign(start$pending$state, start$pending, envir = pending_store)
      output$body <- renderUI(a(href = start$url, "Sign in"))
      return()
    }

    # The callback: look the pending values up once, then forget them.
    pending <- get0(query$state, envir = pending_store, inherits = FALSE)
    if (is.null(pending)) pending <- list() else rm(list = query$state, envir = pending_store)
    id <- tryCatch(
      sso_finish(client, session$clientData$url_search, pending),
      rusckersso_error = function(e) NULL
    )
    updateQueryString("?", mode = "replace") # drop code/state from the address bar
    if (is.null(id)) {
      output$body <- renderUI(p("Sign-in failed."))
      return()
    }
    session$userData$identity <- id # the app's own session from here on
    output$body <- renderUI(p("Hello, ", id$name, if (has_role(id, "fiscal", "gestor")) " (manager)"))
  })
}

shinyApp(ui, server)
```

Keying the pending values by `state` is the simplest honest store, but it
does not tie the sign-in to the browser that started it; binding it to a
random `HttpOnly` cookie closes that gap at the cost of setting a cookie
in front of Shiny (a reverse proxy or a plumber2 route). Pending values
left behind by abandoned sign-ins should be pruned after a few minutes.
A Shiny session ends on reload, so a long-lived login needs the app's own
cookie as well.

## The identity

| Field | Notes |
|---|---|
| `sub` | the person's permanent id — **the key to store** (`NULL` inside the portal) |
| `username` | |
| `name` | full name, or the username when the portal has none (`NULL` inside the portal) |
| `email`, `setor` | `NULL` when absent |
| `groups` | character vector; only the groups that give access to this app when it is restricted |
| `group_roles` | named list: group → character vector of roles |
| `claims` | the raw ID token claims (or, inside the portal, the raw headers) |

Helpers: `has_group(id, g)`, `has_role(id, g, r)`, `roles(id, g)`.

## Errors

Every error has class `rusckersso_error` plus one of:

| Class | When | Fields |
|---|---|---|
| `rusckersso_provider_error` | the portal sent back `error=…` (e.g. `access_denied`) | `error`, `error_description` |
| `rusckersso_state_mismatch` | the callback's `state` is not the stored one; nothing was sent | |
| `rusckersso_issuer_mismatch` | `iss`, or the discovery document's issuer, is not the configured issuer | `actual` |
| `rusckersso_invalid_callback` | no `code`, or a repeated one | |
| `rusckersso_token_refused` | the token endpoint refused the code (`invalid_grant`, `invalid_client`, …) | `error`, `error_description`, `status` |
| `rusckersso_invalid_id_token` | the ID token failed verification | `reason` |
| `rusckersso_http_error` | the portal could not be reached | |

A code works once; do not retry `sso_finish()` with the same callback.

## Tests

```sh
cd clients/r
Rscript -e 'testthat::test_local()'          # unit tests, offline

# end to end, against a throwaway portal (see clients/README.md)
source /tmp/sso-dev/env
Rscript -e 'testthat::test_local()'          # the e2e tests now run too
```
