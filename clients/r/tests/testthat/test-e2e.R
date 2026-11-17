# End to end, against a real portal started by clients/testing/provider.sh.
# Skipped unless its environment file has been sourced (RUSCKER_ISSUER set).

skip_e2e <- function() {
  skip_if(Sys.getenv("RUSCKER_ISSUER") == "", "RUSCKER_ISSUER not set")
  skip_if(Sys.getenv("RUSCKER_TEST_COOKIES") == "", "RUSCKER_TEST_COOKIES not set")
}

e2e_client <- function(client_secret = Sys.getenv("RUSCKER_CLIENT_SECRET"),
                       client_id = Sys.getenv("RUSCKER_CLIENT_ID")) {
  sso_client(
    issuer = Sys.getenv("RUSCKER_ISSUER"),
    client_id = client_id,
    client_secret = client_secret,
    redirect_uri = Sys.getenv("RUSCKER_REDIRECT_URI")
  )
}

# Play the signed-in person's browser: the callback URL's query string.
authorize <- function(url) {
  script <- Sys.getenv("RUSCKER_AUTHORIZE_SH",
                       test_path("..", "..", "..", "testing", "authorize.sh"))
  skip_if_not(file.exists(script), "authorize.sh not found")
  callback <- system2(script, shQuote(url), stdout = TRUE)
  sub("^[^?]*", "", callback[length(callback)])
}

test_that("a full sign-in returns the expected identity", {
  skip_e2e()
  client <- e2e_client()
  start <- sso_start(client)
  id <- sso_finish(client, authorize(start$url), start$pending)
  expect_match(id$sub, ".+")
  expect_equal(id$username, "ana")
  expect_equal(id$name, "Ana Maria de Souza")
  expect_equal(id$email, "ana@example.org")
  expect_equal(id$setor, "GAPE")
  expect_equal(id$groups, "fiscal")
  expect_equal(id$group_roles, list(fiscal = c("analista", "gestor")))
  expect_true(has_role(id, "fiscal", "gestor"))
  expect_equal(id$claims$aud, Sys.getenv("RUSCKER_CLIENT_ID"))
})

test_that("a tampered state is refused before the code is spent", {
  skip_e2e()
  client <- e2e_client()
  start <- sso_start(client)
  query <- authorize(start$url)
  tampered <- start$pending
  tampered$state <- paste0(tampered$state, "x")
  expect_error(sso_finish(client, query, tampered), class = "rusckersso_state_mismatch")
  # The code was never sent: it still works with the right state.
  expect_equal(sso_finish(client, query, start$pending)$username, "ana")
})

test_that("a replayed code is refused", {
  skip_e2e()
  client <- e2e_client()
  start <- sso_start(client)
  query <- authorize(start$url)
  sso_finish(client, query, start$pending)
  err <- expect_error(sso_finish(client, query, start$pending), class = "rusckersso_token_refused")
  expect_equal(err$error, "invalid_grant")
})

test_that("a wrong nonce is refused", {
  skip_e2e()
  client <- e2e_client()
  start <- sso_start(client)
  query <- authorize(start$url)
  wrong <- start$pending
  wrong$nonce <- "not-the-nonce"
  err <- expect_error(sso_finish(client, query, wrong), class = "rusckersso_invalid_id_token")
  expect_match(err$reason, "nonce")
})

test_that("a wrong client secret is refused", {
  skip_e2e()
  start <- sso_start(e2e_client())
  query <- authorize(start$url)
  err <- expect_error(sso_finish(e2e_client(client_secret = "wrong"), query, start$pending),
                      class = "rusckersso_token_refused")
  expect_equal(err$error, "invalid_client")
  expect_equal(err$status, 401L)
})

test_that("an ID token checked as another client's audience is refused", {
  skip_e2e()
  client <- e2e_client()
  start <- sso_start(client)
  query <- httr2::url_query_parse(sub("^\\?", "", authorize(start$url)))
  tokens <- exchange_code(client, query$code, start$pending$code_verifier)
  # Genuine token, right signature, right nonce — but not issued to "other-app".
  other <- e2e_client(client_id = "other-app")
  err <- expect_error(verify_id_token(other, tokens$id_token, start$pending$nonce),
                      class = "rusckersso_invalid_id_token")
  expect_match(err$reason, "audience")
  # The same token passes for the client it was issued to.
  expect_equal(verify_id_token(client, tokens$id_token, start$pending$nonce)$preferred_username, "ana")
})
