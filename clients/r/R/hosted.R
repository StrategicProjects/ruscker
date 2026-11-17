# Half A: apps HOSTED inside the portal.
#
# No OpenID Connect here. The portal's proxy already knows who is signed
# in and, when the app's spec opts in, adds that identity to every request
# (and to the WebSocket handshake) as headers. These functions only read
# and parse them.

#' Read the signed-in person inside a Ruscker-hosted app
#'
#' For an app served by the portal under `/app/<id>/` or `/api/<id>/`.
#' The proxy adds the identity to each request when the app's spec opts in:
#'
#' | Spec setting | Header | Identity field |
#' |---|---|---|
#' | `add-default-http-headers: true` | `X-SP-UserId` | `username` |
#' | | `X-SP-UserGroups` (comma-separated) | `groups` |
#' | `identity-claims: [email]` | `X-Ruscker-User-Email` | `email` |
#' | `identity-claims: [setor]` | `X-Ruscker-User-Setor` | `setor` |
#' | `identity-claims: [roles]` | `X-Ruscker-User-Roles` | `group_roles` |
#'
#' **Why the values can be trusted:** the proxy removes any `X-SP-*` and
#' `X-Ruscker-User-*` header that arrives from outside, so a value the app
#' sees was put there by the portal. That only holds when the app is
#' reachable *only* through the portal.
#'
#' **Read identity per request, never from an environment variable:** one
#' container may serve many people.
#'
#' The headers carry no permanent account id, so `sub` and `name` are
#' always `NULL` here; key on `username`. An app restricted to some groups
#' receives only those groups' roles.
#'
#' @param session For Shiny, the `session` object (its `$request` is read),
#'   or the request environment itself.
#' @param request For plumber2, the `request` argument of a handler (a
#'   `reqres::Request`; anything with a `get_header(name)` method works).
#' @return An identity (see [has_group()]), or `NULL` for an anonymous
#'   visitor — or when the spec has not enabled the headers.
#' @examples
#' # Shiny, inside server():  id <- hosted_identity(session)
#' # plumber2 handler:        id <- hosted_identity_plumber(request)
#' fake <- list(request = list(HTTP_X_SP_USERID = "ana",
#'                             HTTP_X_SP_USERGROUPS = "fiscal,obras"))
#' hosted_identity(fake)$groups
#' @export
hosted_identity <- function(session) {
  # A Shiny session exposes the Rook request environment as `$request`;
  # Rook names headers `HTTP_` + upper case + `-` turned into `_`.
  req <- session[["request"]]
  if (is.null(req)) req <- session # given the request itself
  header <- function(name) {
    key <- paste0("HTTP_", toupper(gsub("-", "_", name, fixed = TRUE)))
    req[[key]] # works on the Rook environment and on a plain list alike
  }
  identity_from_headers(header)
}

#' @rdname hosted_identity
#' @export
hosted_identity_plumber <- function(request) {
  identity_from_headers(function(name) request$get_header(name))
}

# Shared by both: `header(name)` returns the header's value or NULL.
identity_from_headers <- function(header) {
  username <- scalar_or_null(header("X-SP-UserId"))
  if (is.null(username)) {
    return(NULL) # anonymous, or the spec did not opt in
  }
  names_in <- c("X-SP-UserId", "X-SP-UserGroups", "X-Ruscker-User-Email",
                "X-Ruscker-User-Setor", "X-Ruscker-User-Roles")
  raw <- Filter(Negate(is.null), stats::setNames(lapply(names_in, header), names_in))
  new_identity(
    username = username,
    email = header("X-Ruscker-User-Email"),
    setor = header("X-Ruscker-User-Setor"),
    groups = split_groups(header("X-SP-UserGroups")),
    group_roles = parse_roles_header(header("X-Ruscker-User-Roles")),
    claims = raw
  )
}

# `X-SP-UserGroups` is a plain comma-joined list.
split_groups <- function(value) {
  if (is.null(value) || !nzchar(value)) return(character(0))
  parts <- trimws(strsplit(value, ",", fixed = TRUE)[[1]])
  parts[nzchar(parts)]
}

#' Parse the `X-Ruscker-User-Roles` header
#'
#' The value is comma-separated `GROUP:ROLE` entries; `GROUP` is
#' percent-encoded UTF-8 and `ROLE` is limited to `[A-Za-z0-9_.-]`, so
#' after encoding neither part can contain `,` or `:` and the value splits
#' unambiguously. A group name cannot smuggle in an extra entry.
#'
#' @param value The header value, or `NULL`.
#' @return A named list: group -> character vector of roles. Malformed
#'   entries are skipped.
#' @examples
#' parse_roles_header("Fazenda%3A%20n%C3%BAcleo%20A:analista,ops:admin")
#' @export
parse_roles_header <- function(value) {
  out <- stats::setNames(list(), character())
  if (is.null(value) || !nzchar(value)) return(out)
  for (entry in strsplit(value, ",", fixed = TRUE)[[1]]) {
    parts <- strsplit(trimws(entry), ":", fixed = TRUE)[[1]]
    if (length(parts) != 2 || !nzchar(parts[1]) || !nzchar(parts[2])) next
    group <- utils::URLdecode(parts[1])
    Encoding(group) <- "UTF-8" # the bytes decoded are UTF-8
    out[[group]] <- unique(c(out[[group]], parts[2]))
  }
  out
}
