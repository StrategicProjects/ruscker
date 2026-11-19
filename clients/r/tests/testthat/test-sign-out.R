# Signing out, changing the password and the `prompt` option, offline:
# the discovery document is pre-cached (helper-fake-portal.R) and any
# network read fails the test.

no_network <- function() {
  local_mocked_bindings(
    http_get_json = function(url) stop("network must not be used"),
    .env = parent.frame()
  )
}

sign_out_client <- function() {
  client <- fake_client()
  client$discovery$end_session_endpoint <- paste0(fake_issuer, "/oauth/logout")
  client
}

test_that("sso_sign_out_url puts the parameters in order and encodes them", {
  no_network()
  url <- sso_sign_out_url(
    sign_out_client(),
    post_logout_redirect_uri = "https://app.example.org/bye?x=1",
    state = "a b&c?d",
    id_token_hint = "eyJ.hint.sig"
  )
  expect_equal(
    url,
    paste0(
      fake_issuer, "/oauth/logout",
      "?client_id=my-app",
      "&id_token_hint=eyJ.hint.sig",
      "&post_logout_redirect_uri=https%3A%2F%2Fapp.example.org%2Fbye%3Fx%3D1",
      "&state=a%20b%26c%3Fd"
    )
  )
  # And it decodes back to the values given.
  q <- httr2::url_parse(url)$query
  expect_equal(q$state, "a b&c?d")
  expect_equal(q$post_logout_redirect_uri, "https://app.example.org/bye?x=1")
})

test_that("sso_sign_out_url always sends client_id and leaves out what was not given", {
  no_network()
  client <- sign_out_client()
  expect_equal(sso_sign_out_url(client), paste0(fake_issuer, "/oauth/logout?client_id=my-app"))
  q <- httr2::url_parse(sso_sign_out_url(client, state = "s"))$query
  expect_equal(names(q), c("client_id", "state"))
  q <- httr2::url_parse(sso_sign_out_url(client, post_logout_redirect_uri = "https://app.example.org/"))$query
  expect_equal(names(q), c("client_id", "post_logout_redirect_uri"))
})

test_that("a sign-out state longer than 512 characters is refused", {
  no_network()
  client <- sign_out_client()
  expect_no_error(sso_sign_out_url(client, state = strrep("x", 512)))
  expect_error(sso_sign_out_url(client, state = strrep("x", 513)), "at most 512")
  # Counted in characters, not bytes.
  expect_no_error(sso_sign_out_url(client, state = strrep("ç", 512)))
})

test_that("a portal without end_session_endpoint is reported, not guessed", {
  no_network()
  err <- expect_error(sso_sign_out_url(fake_client()), class = "rusckersso_http_error")
  expect_match(conditionMessage(err), "end_session_endpoint")
  expect_s3_class(err, "rusckersso_error")
})

test_that("sso_sign_out_url reads discovery once and then uses the cache", {
  reads <- 0
  local_mocked_bindings(http_get_json = function(url) {
    reads <<- reads + 1
    list(
      issuer = fake_issuer,
      authorization_endpoint = paste0(fake_issuer, "/oauth/authorize"),
      token_endpoint = paste0(fake_issuer, "/oauth/token"),
      jwks_uri = paste0(fake_issuer, "/oauth/jwks"),
      end_session_endpoint = paste0(fake_issuer, "/oauth/logout")
    )
  })
  client <- sso_client(fake_issuer, "my-app", "s", "https://app.example.org/cb")
  sso_sign_out_url(client)
  sso_sign_out_url(client, state = "again")
  expect_equal(reads, 1)
})

test_that("sso_password_url is the fixed path with the encoded client id", {
  no_network()
  expect_equal(
    sso_password_url(fake_client()),
    "https://portal.example.org/admin/account/password?client_id=my-app"
  )
  expect_equal(
    sso_password_url(fake_client(client_id = "my app&x")),
    "https://portal.example.org/admin/account/password?client_id=my%20app%26x"
  )
  # No discovery at all: works on a client that never read it.
  client <- sso_client("https://portal.example.org/sub/", "a", "s", "https://app.example.org/")
  expect_equal(sso_password_url(client), "https://portal.example.org/sub/admin/account/password?client_id=a")
})

test_that("sso_start sends prompt=none only when asked", {
  no_network()
  plain <- sso_start(fake_client())$url
  expect_null(httr2::url_parse(plain)$query$prompt)
  expect_false(grepl("prompt", plain, fixed = TRUE))
  quiet <- httr2::url_parse(sso_start(fake_client(), prompt = "none")$url)$query
  expect_equal(quiet$prompt, "none")
  expect_error(sso_start(fake_client(), prompt = c("none", "login")), "prompt")
})
