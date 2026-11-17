# ID token verification, offline, against a locally generated key.

verify <- function(token, client = fake_client(), nonce = "the-nonce") {
  rusckersso:::verify_id_token(client, token, nonce)
}

expect_invalid <- function(expr, reason_pattern) {
  err <- expect_error(expr, class = "rusckersso_invalid_id_token")
  expect_match(err$reason, reason_pattern)
}

test_that("a good token verifies", {
  expect_equal(verify(mint())$sub, "5966f47e-a879-4aee-8588-69e44be72525")
})

test_that("only ES256 is accepted, whatever the token says", {
  hs <- jose::jwt_encode_hmac(
    structure(fake_claims(), class = c("jwt_claim", "list")),
    secret = charToRaw("s3cret"), header = list(kid = fake_kid)
  )
  expect_invalid(verify(hs), "HS256")
  # An unsigned token ("alg": "none").
  none <- paste(jose::base64url_encode(charToRaw('{"alg":"none","kid":"test-key-1"}')),
                strsplit(mint(), ".", fixed = TRUE)[[1]][2], "", sep = ".")
  expect_invalid(verify(none), "none|not a signed")
})

test_that("a token signed by another key is refused", {
  other <- openssl::ec_keygen("P-256")
  expect_invalid(verify(mint(key = other)), "signature")
})

test_that("an unknown kid triggers one refresh of the key set", {
  calls <- 0
  local_mocked_bindings(http_get_json = function(url) {
    calls <<- calls + 1
    list(keys = list(fake_jwk(kid = "rotated")))
  })
  expect_equal(verify(mint(kid = "rotated"))$preferred_username, "ana")
  expect_equal(calls, 1)
  # Still unknown after the refresh: refused, no second refresh loop.
  calls <- 0
  expect_invalid(verify(mint(kid = "never")), "unknown key id")
  expect_equal(calls, 1)
})

test_that("wrong issuer, audience, nonce and expired tokens are refused", {
  expect_invalid(verify(mint(fake_claims(iss = "https://evil.example.org"))), "issuer")
  expect_invalid(verify(mint(fake_claims(aud = "another-app"))), "audience")
  expect_invalid(verify(mint(), client = fake_client("another-app")), "audience")
  expect_invalid(verify(mint(), nonce = "other-nonce"), "nonce")
  expect_invalid(verify(mint(fake_claims(nonce = NULL))), "nonce")
  expect_invalid(verify(mint(fake_claims(exp = as.integer(Sys.time()) - 3600L))), "expired")
  expect_invalid(verify(mint(fake_claims(exp = NULL))), "exp")
})

test_that("garbage is refused with a reason", {
  expect_invalid(verify("not-a-jwt"), "signed JWT")
  expect_invalid(verify(NULL), "missing")
})
