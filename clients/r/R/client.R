# Half B: apps running OUTSIDE the portal sign people in through its
# OpenID Connect provider (authorization code flow + PKCE).
#
# The flow, in the app's words:
#   1. sso_start()  -> send the browser to `url`, keep `pending` server-side;
#   2. the portal signs the person in and redirects back with `code`;
#   3. sso_finish() -> checks the callback, trades the code for an ID token
#      at the portal (server to server, with the client secret), verifies
#      the token and returns the identity.

default_scopes <- "openid profile email groups roles setor"

#' Configure a Ruscker sign-in client
#'
#' Nothing is fetched yet: the discovery document and the signing keys are
#' read from the portal on first use and kept in the client object.
#'
#' @param issuer The portal's address exactly as its discovery document
#'   states it, e.g. `"https://portal.example.org"` (include the subpath
#'   when the portal is mounted under one). Compared byte for byte.
#' @param client_id The system's id, as registered on the portal.
#' @param client_secret The secret shown once at registration. Keep it out
#'   of code: read it from an environment variable.
#' @param redirect_uri The app's callback URL, exactly as registered.
#' @param scopes Space-separated scopes. The default asks for everything
#'   the identity can carry.
#' @return A `rusckersso_client` (an environment, so the cache is shared by
#'   every copy of it).
#' @examples
#' client <- sso_client(
#'   issuer = "https://portal.example.org",
#'   client_id = "my-app",
#'   client_secret = "from-an-environment-variable",
#'   redirect_uri = "https://app.example.org/"
#' )
#' @export
sso_client <- function(issuer, client_id, client_secret, redirect_uri,
                       scopes = default_scopes) {
  for (arg in c("issuer", "client_id", "client_secret", "redirect_uri", "scopes")) {
    v <- get(arg)
    if (!is.character(v) || length(v) != 1 || !nzchar(v)) {
      rlang::abort(sprintf("`%s` must be a single non-empty string.", arg))
    }
  }
  client <- new.env(parent = emptyenv())
  client$issuer <- issuer
  client$client_id <- client_id
  client$client_secret <- client_secret
  client$redirect_uri <- redirect_uri
  client$scopes <- scopes
  client$discovery <- NULL # filled by get_discovery()
  client$jwks <- NULL # filled by get_jwks()
  class(client) <- "rusckersso_client"
  client
}

#' @export
print.rusckersso_client <- function(x, ...) {
  cat("<rusckersso client>\n")
  cat("  issuer:       ", x$issuer, "\n", sep = "")
  cat("  client_id:    ", x$client_id, "\n", sep = "")
  cat("  redirect_uri: ", x$redirect_uri, "\n", sep = "")
  cat("  scopes:       ", x$scopes, "\n", sep = "")
  invisible(x) # the secret is never printed
}

#' Start a sign-in
#'
#' Creates fresh `state`, `nonce` and PKCE `code_verifier` values (32 bytes
#' from OpenSSL's random source each) and the authorization URL.
#'
#' Send the person's browser to `url`. Keep `pending` **on the server** (or
#' in a short-lived `HttpOnly` cookie) until they come back; it is one
#' plain list, so it can be saved however the app likes. Use it once.
#'
#' @param client A client from [sso_client()].
#' @return A list with `url` and `pending` (a list of `state`, `nonce` and
#'   `code_verifier`).
#' @export
sso_start <- function(client) {
  conf <- get_discovery(client)
  pending <- list(
    state = random_token(),
    nonce = random_token(),
    code_verifier = random_token()
  )
  url <- httr2::url_modify_query(
    conf$authorization_endpoint,
    response_type = "code",
    client_id = client$client_id,
    redirect_uri = client$redirect_uri,
    scope = client$scopes,
    state = pending$state,
    nonce = pending$nonce,
    code_challenge = pkce_challenge(pending$code_verifier),
    code_challenge_method = "S256"
  )
  list(url = url, pending = pending)
}

#' Finish a sign-in
#'
#' Runs the checks in this order, stopping at the first failure:
#'
#' 1. an `error` parameter -> `rusckersso_provider_error`;
#' 2. `state` must equal `pending$state` -> `rusckersso_state_mismatch`;
#' 3. `iss`, when present, must equal the issuer ->
#'    `rusckersso_issuer_mismatch`;
#' 4. the code is exchanged at the token endpoint (client secret over HTTP
#'    Basic, the `redirect_uri`, the `code_verifier`) ->
#'    `rusckersso_token_refused`;
#' 5. the ID token is verified: ES256 only, signature against the portal's
#'    keys, `iss`, `aud`, `exp`, `nonce` -> `rusckersso_invalid_id_token`;
#' 6. the identity is returned.
#'
#' Steps 1-3 happen before any network call. A code works once: never
#' call this twice with the same callback.
#'
#' @param client A client from [sso_client()].
#' @param query The callback's query: a string (`"?code=...&state=..."`, as
#'   in Shiny's `session$clientData$url_search`) or a named list.
#' @param pending The `pending` value [sso_start()] returned.
#' @return An identity (see [has_group()]). The raw ID token claims are in
#'   `$claims`.
#' @export
sso_finish <- function(client, query, pending) {
  query <- normalise_query(query)

  # 1. The portal reports a refusal (e.g. access_denied) as `error`.
  # (`[[` rather than `$`: `$` on a list would partially match
  # `error_description` when `error` is absent.)
  error <- query[["error"]]
  if (!is.null(error)) {
    sso_abort(
      "provider_error",
      sprintf("The portal refused the sign-in: %s.", error[[1]]),
      error = error[[1]],
      error_description = query[["error_description"]]
    )
  }

  # 2. state ties this callback to the sign-in this app started.
  # A missing or repeated `state` is a mismatch too.
  state <- unlist(query[names(query) == "state"], use.names = FALSE)
  if (!constant_time_equal(state, pending[["state"]])) {
    sso_abort("state_mismatch", "The callback's `state` does not match the stored one.")
  }

  # 3. iss (RFC 9207) says which portal sent the person back.
  if (!is.null(query[["iss"]])) {
    iss <- single_param(query, "iss")
    if (!identical(iss, client$issuer)) {
      sso_abort("issuer_mismatch",
                sprintf("The callback came from issuer `%s`, not `%s`.", iss, client$issuer),
                actual = iss)
    }
  }

  # 4. and 5.
  code <- single_param(query, "code")
  tokens <- exchange_code(client, code, pending[["code_verifier"]])
  claims <- verify_id_token(client, tokens$id_token, pending[["nonce"]])

  # 6.
  identity_from_claims(claims)
}

identity_from_claims <- function(claims) {
  new_identity(
    sub = claims$sub,
    username = claims$preferred_username,
    name = claims$name,
    email = claims$email,
    setor = claims$setor,
    groups = claims$groups,
    group_roles = claims$group_roles,
    claims = claims
  )
}

# ---- step 4: the code exchange --------------------------------------------

# POST the code to the token endpoint. Never retried: the portal spends the
# code on the first attempt by an authenticated client.
exchange_code <- function(client, code, code_verifier) {
  conf <- get_discovery(client)
  # RFC 6749 section 2.3.1: id and secret are form-urlencoded before Basic.
  # `repeated = TRUE`: without it URLencode() returns a value that already
  # contains "%XX" untouched, and the portal would decode it to something else.
  enc <- function(x) utils::URLencode(x, reserved = TRUE, repeated = TRUE)
  req <- httr2::request(conf$token_endpoint) |>
    httr2::req_auth_basic(enc(client$client_id), enc(client$client_secret)) |>
    httr2::req_body_form(
      grant_type = "authorization_code",
      code = code,
      redirect_uri = client$redirect_uri,
      code_verifier = code_verifier
    ) |>
    httr2::req_error(is_error = function(resp) FALSE) |>
    safe_request()
  resp <- tryCatch(
    httr2::req_perform(req),
    error = function(e) {
      sso_abort("http_error", "Could not reach the portal's token endpoint.", parent = e)
    }
  )
  body <- tryCatch(httr2::resp_body_json(resp), error = function(e) list())
  status <- httr2::resp_status(resp)
  if (status != 200 || is.null(body$id_token)) {
    err <- if (is.character(body$error)) {
      body$error
    } else if (status == 200) {
      "missing_id_token" # an answer without an ID token is unusable
    } else {
      sprintf("http_%d", status)
    }
    sso_abort(
      "token_refused",
      sprintf("The portal refused the code: %s.", err),
      error = err,
      error_description = body$error_description,
      status = status
    )
  }
  body
}

# ---- discovery and keys ----------------------------------------------------

# GET a URL and parse its JSON. One place for HTTP reads, so the unit
# tests can stand in for the network by mocking this function.
http_get_json <- function(url) {
  resp <- tryCatch(
    httr2::request(url) |> safe_request() |> httr2::req_perform(),
    error = function(e) sso_abort("http_error", sprintf("Could not read %s.", url), parent = e)
  )
  # A non-JSON answer is the same failure for the caller: keep it inside
  # the package's error class.
  tryCatch(
    httr2::resp_body_json(resp, simplifyVector = FALSE),
    error = function(e) sso_abort("http_error", sprintf("%s did not answer with JSON.", url), parent = e)
  )
}

# Every request: give up after 10 seconds (Shiny is one process — a
# stalled portal would freeze the app for everyone) and do not follow
# redirects (a redirect must not carry the code to another host).
safe_request <- function(req) {
  req |>
    httr2::req_timeout(10) |>
    httr2::req_options(followlocation = FALSE)
}

# The discovery document, read once. Its `issuer` must be the configured
# one: that is what makes the endpoints in it trustworthy.
get_discovery <- function(client) {
  if (is.null(client$discovery)) {
    base <- sub("/+$", "", client$issuer)
    conf <- http_get_json(paste0(base, "/.well-known/openid-configuration"))
    if (!identical(conf$issuer, client$issuer)) {
      sso_abort("issuer_mismatch",
                sprintf("The discovery document names issuer `%s`, not `%s`.",
                        format(conf$issuer), client$issuer),
                actual = conf$issuer)
    }
    for (k in c("authorization_endpoint", "token_endpoint", "jwks_uri")) {
      if (!is.character(conf[[k]])) {
        sso_abort("http_error", sprintf("The discovery document has no `%s`.", k))
      }
    }
    client$discovery <- conf
  }
  client$discovery
}

# The JSON Web Key Set, cached. `refresh = TRUE` forces a new read.
get_jwks <- function(client, refresh = FALSE) {
  if (refresh || is.null(client$jwks)) {
    client$jwks <- http_get_json(get_discovery(client)$jwks_uri)
  }
  client$jwks
}

# ---- small helpers ---------------------------------------------------------

# 32 random bytes from OpenSSL, base64url without padding: 43 characters,
# which also satisfies PKCE's verifier alphabet and length (43-128).
random_token <- function() {
  jose::base64url_encode(openssl::rand_bytes(32))
}

# PKCE S256: BASE64URL(SHA256(verifier)).
pkce_challenge <- function(verifier) {
  jose::base64url_encode(unclass(openssl::sha256(charToRaw(verifier))))
}

# Compare two strings without stopping at the first different byte, so the
# time taken does not reveal how much of a guess was right. R has no
# built-in for this; this is the usual XOR-accumulate.
constant_time_equal <- function(a, b) {
  if (!is.character(a) || !is.character(b) || length(a) != 1 || length(b) != 1) return(FALSE)
  # An empty value never matches: lost pending values must not "equal"
  # an absent parameter.
  if (is.na(a) || is.na(b) || !nzchar(a) || !nzchar(b)) return(FALSE)
  x <- charToRaw(enc2utf8(a))
  y <- charToRaw(enc2utf8(b))
  if (length(x) != length(y)) return(FALSE)
  diff <- 0L
  for (i in seq_along(x)) diff <- bitwOr(diff, bitwXor(as.integer(x[i]), as.integer(y[i])))
  diff == 0L
}

# Accept "?a=1&b=2", "a=1&b=2" or a named list (e.g. shiny::parseQueryString()).
normalise_query <- function(query) {
  if (is.character(query) && length(query) == 1) {
    q <- sub("^\\?", "", query)
    return(if (nzchar(q)) httr2::url_query_parse(q) else list())
  }
  if (is.list(query)) return(query)
  rlang::abort("`query` must be a query string or a named list.")
}

# A callback parameter that must be there exactly once. A repeated
# parameter is refused rather than guessed at.
single_param <- function(query, name) {
  hits <- query[names(query) == name]
  values <- unlist(hits, use.names = FALSE)
  if (length(values) != 1 || !is.character(values) || !nzchar(values)) {
    sso_abort("invalid_callback",
              sprintf("The callback must carry exactly one `%s`.", name))
  }
  values
}
