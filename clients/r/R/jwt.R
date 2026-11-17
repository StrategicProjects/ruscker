# Step 5 of sso_finish(): verify the ID token.
#
# The signature itself is checked by the 'jose' package. What 'jose' does
# NOT do is decide which algorithm is acceptable (it follows the token's
# own header) or check iss / aud / nonce, so those are done here:
#
#   * the algorithm is pinned to ES256 BEFORE anything is verified. Taking
#     the algorithm from the token would let an attacker pick a weaker one;
#   * the key is the portal's key named by the token's `kid`, and must be
#     an EC P-256 key;
#   * then iss, aud, exp and nonce.

# Clock skew tolerated on `exp`, in seconds (the same as 'jose' uses).
exp_leeway <- 60

verify_id_token <- function(client, id_token, nonce) {
  bad <- function(reason) {
    sso_abort("invalid_id_token", sprintf("The ID token is invalid: %s.", reason),
              reason = reason, call = NULL)
  }
  if (!is.character(id_token) || length(id_token) != 1) bad("missing")

  # Read the JOSE header (not yet trusted) only to pin the algorithm and
  # find the key id.
  parts <- strsplit(id_token, ".", fixed = TRUE)[[1]]
  if (length(parts) != 3) bad("not a signed JWT")
  header <- tryCatch(
    jsonlite::fromJSON(rawToChar(jose::base64url_decode(parts[1])), simplifyVector = FALSE),
    error = function(e) NULL
  )
  if (!is.list(header)) bad("unreadable header")
  if (!identical(header$alg, "ES256")) {
    bad(sprintf("algorithm `%s` is not accepted, only ES256", format(header$alg)))
  }
  kid <- header$kid
  if (!is.character(kid) || length(kid) != 1) bad("no key id (`kid`) in the header")

  # Find the key; an unknown kid triggers one refresh (keys can rotate).
  jwk <- find_key(get_jwks(client), kid)
  if (is.null(jwk)) jwk <- find_key(get_jwks(client, refresh = TRUE), kid)
  if (is.null(jwk)) bad(sprintf("unknown key id `%s`", kid))
  if (!identical(jwk$kty, "EC") || !identical(jwk$crv, "P-256") ||
      (!is.null(jwk$alg) && !identical(jwk$alg, "ES256"))) {
    bad("the portal's key is not an ES256 (EC P-256) key")
  }
  key <- tryCatch(jose::read_jwk(jwk), error = function(e) NULL)
  if (is.null(key)) bad("the portal's key could not be read")

  # The signature (and, as a side effect, jose's own exp check).
  claims <- tryCatch(
    jose::jwt_decode_sig(id_token, key),
    error = function(e) {
      msg <- conditionMessage(e)
      if (grepl("expired", msg, ignore.case = TRUE)) bad("expired")
      bad(paste("signature check failed:", msg))
    }
  )
  claims <- unclass(claims)

  # The claims.
  if (!identical(claims$iss, client$issuer)) bad("wrong issuer (`iss`)")
  aud <- claims$aud
  if (!is.character(aud) || !(client$client_id %in% aud)) bad("wrong audience (`aud`)")
  if (!is.numeric(claims$exp) || length(claims$exp) != 1) bad("no expiry (`exp`)")
  if (claims$exp < as.numeric(Sys.time()) - exp_leeway) bad("expired")
  if (!constant_time_equal(claims$nonce, nonce)) bad("wrong `nonce`")
  if (!is.character(claims$sub) || length(claims$sub) != 1 || !nzchar(claims$sub)) {
    bad("no subject (`sub`)")
  }
  claims
}

find_key <- function(jwks, kid) {
  for (k in jwks$keys) if (identical(k$kid, kid)) return(k)
  NULL
}
