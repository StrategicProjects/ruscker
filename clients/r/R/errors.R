# Classed errors.
#
# Every failure this package raises is an R condition with the class
# `rusckersso_error` plus one more specific class, so an app can react to
# each case with `tryCatch(..., rusckersso_state_mismatch = function(e) ...)`
# instead of matching on message text (which may change).

#' Errors raised by rusckersso
#'
#' All errors inherit from `rusckersso_error`. The specific classes are:
#'
#' * `rusckersso_provider_error` — the portal sent the person back with an
#'   `error` parameter (for example `access_denied`). Fields: `error`,
#'   `error_description`.
#' * `rusckersso_state_mismatch` — the callback's `state` is not the one
#'   stored at the start. Nothing was sent to the portal.
#' * `rusckersso_issuer_mismatch` — the callback's `iss`, or the discovery
#'   document's `issuer`, is not the configured issuer. Field: `actual`.
#' * `rusckersso_invalid_callback` — the callback lacks `code` or `state`,
#'   or repeats one of them.
#' * `rusckersso_token_refused` — the token endpoint refused the code.
#'   Fields: `error` (the provider's code, such as `invalid_grant` or
#'   `invalid_client`), `error_description`, `status`.
#' * `rusckersso_invalid_id_token` — the ID token failed verification.
#'   Field: `reason`.
#' * `rusckersso_http_error` — the portal could not be reached, or answered
#'   discovery / JWKS with something unusable.
#'
#' @name rusckersso-errors
NULL

# Raise one of the errors above. `subclass` is the part after `rusckersso_`.
sso_abort <- function(subclass, message, ..., call = rlang::caller_env()) {
  rlang::abort(
    message,
    class = c(paste0("rusckersso_", subclass), "rusckersso_error"),
    ...,
    call = call
  )
}
