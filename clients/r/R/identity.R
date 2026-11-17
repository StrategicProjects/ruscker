# The identity object shared by both halves of the package: what an
# external app gets from `sso_finish()` and what a hosted app gets from
# `hosted_identity()` / `hosted_identity_plumber()`.

# Build an identity. Everything is normalised here, once, so the helpers
# and the app never have to wonder whether `groups` is `NULL`, `list()` or
# a character vector: it is always a character vector, and `group_roles`
# is always a named list of character vectors.
new_identity <- function(sub = NULL, username = NULL, name = NULL,
                         email = NULL, setor = NULL, groups = NULL,
                         group_roles = NULL, claims = list()) {
  roles_map <- lapply(as.list(group_roles), function(r) as.character(unlist(r)))
  if (length(roles_map) == 0) roles_map <- stats::setNames(list(), character())
  structure(
    list(
      sub = scalar_or_null(sub),
      username = scalar_or_null(username),
      name = scalar_or_null(name),
      email = scalar_or_null(email),
      setor = scalar_or_null(setor),
      groups = as.character(unlist(groups)),
      group_roles = roles_map,
      claims = claims
    ),
    class = "rusckersso_identity"
  )
}

# A claim is either one non-empty string or absent (NULL).
scalar_or_null <- function(x) {
  if (is.null(x) || length(x) != 1 || is.na(x) || !nzchar(x)) NULL else as.character(x)
}

#' Ask an identity about its groups and roles
#'
#' * `has_group(identity, group)` — is the person in `group`?
#' * `has_role(identity, group, role)` — does the person hold `role` in
#'   `group`?
#' * `roles(identity, group)` — every role the person holds in `group`
#'   (`character(0)` when none).
#'
#' These only report what the portal said. What a group or a role allows
#' inside your app is your app's decision.
#'
#' @param identity An identity from [sso_finish()], [hosted_identity()] or
#'   [hosted_identity_plumber()].
#' @param group A group name.
#' @param role A role name.
#' @return `TRUE`/`FALSE` for `has_group()` and `has_role()`; a character
#'   vector for `roles()`.
#' @examples
#' # A stand-in for a Shiny session inside the portal.
#' session <- list(request = list(
#'   HTTP_X_SP_USERID = "ana",
#'   HTTP_X_SP_USERGROUPS = "fiscal",
#'   HTTP_X_RUSCKER_USER_ROLES = "fiscal:analista,fiscal:gestor"
#' ))
#' id <- hosted_identity(session)
#' has_group(id, "fiscal")
#' has_role(id, "fiscal", "gestor")
#' roles(id, "fiscal")
#' @export
has_group <- function(identity, group) {
  isTRUE(group %in% identity$groups)
}

#' @rdname has_group
#' @export
has_role <- function(identity, group, role) {
  isTRUE(role %in% roles(identity, group))
}

#' @rdname has_group
#' @export
roles <- function(identity, group) {
  r <- identity$group_roles[[group]]
  if (is.null(r)) character(0) else r
}

#' @export
print.rusckersso_identity <- function(x, ...) {
  show <- function(v) if (is.null(v)) "<absent>" else v
  cat("<rusckersso identity>\n")
  cat("  sub:      ", show(x$sub), "\n", sep = "")
  cat("  username: ", show(x$username), "\n", sep = "")
  cat("  name:     ", show(x$name), "\n", sep = "")
  cat("  email:    ", show(x$email), "\n", sep = "")
  cat("  setor:    ", show(x$setor), "\n", sep = "")
  cat("  groups:   ", paste(x$groups, collapse = ", "), "\n", sep = "")
  for (g in names(x$group_roles)) {
    cat("  roles in ", g, ": ", paste(x$group_roles[[g]], collapse = ", "), "\n", sep = "")
  }
  invisible(x)
}
