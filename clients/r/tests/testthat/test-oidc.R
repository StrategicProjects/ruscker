# Half B, offline: the client object has its discovery document and keys
# pre-cached (helper-fake-portal.R), and the token exchange is mocked.

test_that("sso_start builds the authorization URL with PKCE S256", {
  client <- fake_client()
  start <- sso_start(client)
  q <- httr2::url_parse(start$url)$query
  expect_true(startsWith(start$url, paste0(fake_issuer, "/oauth/authorize?")))
  expect_equal(q$response_type, "code")
  expect_equal(q$client_id, "my-app")
  expect_equal(q$redirect_uri, "https://app.example.org/cb")
  expect_equal(q$scope, "openid profile email groups roles setor")
  expect_equal(q$code_challenge_method, "S256")
  expect_equal(q$state, start$pending$state)
  expect_equal(q$nonce, start$pending$nonce)
  # The challenge is BASE64URL(SHA256(verifier)), and the verifier never
  # appears in the URL.
  expect_equal(q$code_challenge, rusckersso:::pkce_challenge(start$pending$code_verifier))
  expect_false(grepl(start$pending$code_verifier, start$url, fixed = TRUE))
})

test_that("the PKCE challenge matches the RFC 7636 example", {
  expect_equal(
    rusckersso:::pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
    "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
  )
})

test_that("state, nonce and verifier are fresh, 43 chars of base64url", {
  a <- sso_start(fake_client())$pending
  b <- sso_start(fake_client())$pending
  for (v in c(a, b)) expect_match(v, "^[A-Za-z0-9_-]{43}$")
  expect_length(unique(c(unlist(a), unlist(b))), 6)
})

test_that("an error callback is reported with its code", {
  err <- expect_error(
    sso_finish(fake_client(), "?error=access_denied&state=x", list(state = "x")),
    class = "rusckersso_provider_error"
  )
  expect_equal(err$error, "access_denied")
  expect_s3_class(err, "rusckersso_error")
})

test_that("an error_description alone is not mistaken for an error", {
  # `$` partial matching would have read `error_description` as `error`.
  expect_error(
    sso_finish(fake_client(), list(error_description = "x", state = "bad"), list(state = "s")),
    class = "rusckersso_state_mismatch"
  )
})

test_that("a different, missing or repeated state is refused before any network call", {
  local_mocked_bindings(exchange_code = function(...) stop("network must not be used"))
  client <- fake_client()
  pending <- list(state = "abc", nonce = "n", code_verifier = "v")
  expect_error(sso_finish(client, "?code=c&state=abd", pending), class = "rusckersso_state_mismatch")
  expect_error(sso_finish(client, "?code=c", pending), class = "rusckersso_state_mismatch")
  expect_error(sso_finish(client, list(code = "c", state = "abc", state = "abc"), pending),
               class = "rusckersso_state_mismatch")
})

test_that("an iss from another portal is refused", {
  local_mocked_bindings(exchange_code = function(...) stop("network must not be used"))
  err <- expect_error(
    sso_finish(fake_client(), list(code = "c", state = "s", iss = "https://evil.example.org"),
               list(state = "s")),
    class = "rusckersso_issuer_mismatch"
  )
  expect_equal(err$actual, "https://evil.example.org")
})

test_that("a callback without a code is refused", {
  expect_error(sso_finish(fake_client(), "?state=s", list(state = "s")),
               class = "rusckersso_invalid_callback")
})

test_that("a discovery document naming another issuer is refused", {
  local_mocked_bindings(http_get_json = function(url) list(issuer = "https://other.example.org"))
  client <- sso_client(fake_issuer, "my-app", "s", "https://app.example.org/cb")
  expect_error(sso_start(client), class = "rusckersso_issuer_mismatch")
})

test_that("a full offline sign-in returns the identity", {
  local_mocked_bindings(exchange_code = function(client, code, verifier) {
    expect_equal(code, "the-code")
    expect_equal(verifier, "the-verifier")
    list(id_token = mint())
  })
  pending <- list(state = "s", nonce = "the-nonce", code_verifier = "the-verifier")
  id <- sso_finish(fake_client(), paste0("?code=the-code&state=s&iss=", utils::URLencode(fake_issuer, reserved = TRUE)), pending)
  expect_equal(id$sub, "5966f47e-a879-4aee-8588-69e44be72525")
  expect_equal(id$username, "ana")
  expect_equal(id$name, "Ana Maria de Souza")
  expect_equal(id$email, "ana@example.org")
  expect_equal(id$setor, "GAPE")
  expect_equal(id$groups, "fiscal")
  expect_equal(id$group_roles, list(fiscal = c("analista", "gestor")))
  expect_true(has_group(id, "fiscal"))
  expect_false(has_group(id, "obras"))
  expect_true(has_role(id, "fiscal", "gestor"))
  expect_false(has_role(id, "fiscal", "admin"))
  expect_equal(roles(id, "obras"), character(0))
  expect_equal(id$claims$aud, "my-app") # raw claims stay available
})

test_that("missing optional claims become NULL / empty", {
  local_mocked_bindings(exchange_code = function(...) {
    list(id_token = mint(fake_claims(email = NULL, setor = NULL, groups = NULL,
                                     group_roles = NULL, name = NULL)))
  })
  id <- sso_finish(fake_client(), "?code=c&state=s", list(state = "s", nonce = "the-nonce"))
  expect_null(id$email)
  expect_null(id$setor)
  expect_null(id$name)
  expect_equal(id$groups, character(0))
  expect_equal(length(id$group_roles), 0)
  expect_false(has_group(id, "fiscal"))
})

test_that("a token endpoint refusal carries the provider's error code", {
  local_mocked_bindings(exchange_code = function(...) {
    rusckersso:::sso_abort("token_refused", "refused", error = "invalid_grant", status = 400L)
  })
  err <- expect_error(sso_finish(fake_client(), "?code=c&state=s", list(state = "s")),
                      class = "rusckersso_token_refused")
  expect_equal(err$error, "invalid_grant")
})

test_that("constant_time_equal compares exactly", {
  cte <- rusckersso:::constant_time_equal
  expect_true(cte("abc", "abc"))
  expect_false(cte("abc", "abd"))
  expect_false(cte("abc", "abcd"))
  expect_false(cte(NULL, "abc"))
  expect_false(cte("abc", NULL))
  expect_false(cte(c("a", "a"), "a"))
})

test_that("the secret is not printed", {
  out <- capture.output(print(fake_client()))
  expect_false(any(grepl("s3cret", out, fixed = TRUE)))
})

test_that("empty values never match and encoded secrets are re-encoded", {
  expect_false(rusckersso:::constant_time_equal("", ""))
  expect_false(rusckersso:::constant_time_equal("", "x"))
  expect_true(rusckersso:::constant_time_equal("x", "x"))
  # A value that already looks percent-encoded must still be encoded.
  expect_equal(utils::URLencode("a%41b", reserved = TRUE, repeated = TRUE), "a%2541b")
})
