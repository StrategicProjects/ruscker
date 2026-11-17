# An offline stand-in for the portal: a signing key, a client whose
# discovery document and key set are already cached (so nothing is
# fetched), and a way to mint ID tokens.

fake_issuer <- "https://portal.example.org"

fake_key <- openssl::ec_keygen("P-256")
fake_kid <- "test-key-1"

fake_jwk <- function(key = fake_key, kid = fake_kid) {
  jwk <- jsonlite::fromJSON(jose::write_jwk(key$pubkey), simplifyVector = FALSE)
  jwk$kid <- kid
  jwk$alg <- "ES256"
  jwk$use <- "sig"
  jwk
}

fake_client <- function(client_id = "my-app") {
  client <- sso_client(
    issuer = fake_issuer,
    client_id = client_id,
    client_secret = "s3cret",
    redirect_uri = "https://app.example.org/cb"
  )
  client$discovery <- list(
    issuer = fake_issuer,
    authorization_endpoint = paste0(fake_issuer, "/oauth/authorize"),
    token_endpoint = paste0(fake_issuer, "/oauth/token"),
    jwks_uri = paste0(fake_issuer, "/oauth/jwks")
  )
  client$jwks <- list(keys = list(fake_jwk()))
  client
}

# The claims the real portal sends for the test person, overridable.
fake_claims <- function(...) {
  base <- list(
    iss = fake_issuer,
    aud = "my-app",
    sub = "5966f47e-a879-4aee-8588-69e44be72525",
    iat = as.integer(Sys.time()),
    exp = as.integer(Sys.time()) + 300L,
    nonce = "the-nonce",
    preferred_username = "ana",
    name = "Ana Maria de Souza",
    email = "ana@example.org",
    setor = "GAPE",
    groups = list("fiscal"),
    group_roles = list(fiscal = list("analista", "gestor"))
  )
  utils::modifyList(base, list(...), keep.null = FALSE)
}

mint <- function(claims = fake_claims(), key = fake_key, kid = fake_kid) {
  claim <- structure(claims, class = c("jwt_claim", "list"))
  jose::jwt_encode_sig(claim, key, header = list(kid = kid))
}
