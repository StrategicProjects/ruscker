# Half A: reading the proxy's identity headers. Fake request objects only.

shiny_session <- function(...) {
  # A Shiny session's `$request` is a Rook environment.
  list(request = list2env(list(...)))
}

plumber_request <- function(headers) {
  # plumber2's reqres::Request: get_header() is case-insensitive, NULL if absent.
  headers <- as.list(headers)
  names(headers) <- tolower(names(headers))
  list(get_header = function(name) headers[[tolower(name)]])
}

test_that("a Shiny session with every header gives the full identity", {
  id <- hosted_identity(shiny_session(
    HTTP_X_SP_USERID = "ana",
    HTTP_X_SP_USERGROUPS = "fiscal,obras",
    HTTP_X_RUSCKER_USER_EMAIL = "ana@example.org",
    HTTP_X_RUSCKER_USER_SETOR = "GAPE",
    HTTP_X_RUSCKER_USER_ROLES = "fiscal:analista,fiscal:gestor,obras:leitor"
  ))
  expect_s3_class(id, "rusckersso_identity")
  expect_equal(id$username, "ana")
  expect_null(id$sub) # the headers carry no permanent id
  expect_equal(id$email, "ana@example.org")
  expect_equal(id$setor, "GAPE")
  expect_equal(id$groups, c("fiscal", "obras"))
  expect_equal(id$group_roles, list(fiscal = c("analista", "gestor"), obras = "leitor"))
  expect_true(has_role(id, "fiscal", "gestor"))
  expect_equal(id$claims[["X-SP-UserId"]], "ana")
})

test_that("the Rook request itself is accepted in place of the session", {
  req <- list2env(list(HTTP_X_SP_USERID = "ana"))
  expect_equal(hosted_identity(req)$username, "ana")
})

test_that("plumber2 reads the same headers", {
  id <- hosted_identity_plumber(plumber_request(c(
    "X-SP-UserId" = "ana",
    "X-SP-UserGroups" = "fiscal",
    "X-Ruscker-User-Roles" = "fiscal:analista"
  )))
  expect_equal(id$username, "ana")
  expect_equal(id$groups, "fiscal")
  expect_equal(roles(id, "fiscal"), "analista")
  expect_null(id$email)
  expect_null(id$setor)
})

test_that("an anonymous visitor (no X-SP-UserId) is NULL", {
  expect_null(hosted_identity(shiny_session()))
  expect_null(hosted_identity(shiny_session(HTTP_X_SP_USERID = "")))
  expect_null(hosted_identity_plumber(plumber_request(c("X-SP-UserGroups" = "fiscal"))))
})

test_that("only the username present gives empty groups and roles", {
  id <- hosted_identity(shiny_session(HTTP_X_SP_USERID = "ana"))
  expect_equal(id$groups, character(0))
  expect_equal(length(id$group_roles), 0)
  expect_false(has_group(id, "fiscal"))
  expect_equal(roles(id, "fiscal"), character(0))
})

test_that("the roles header decodes percent-encoded UTF-8 group names", {
  # The portal's own worked example.
  parsed <- parse_roles_header(
    "Fazenda%3A%20n%C3%BAcleo%20A:analista,Fazenda%3A%20n%C3%BAcleo%20A:gestor"
  )
  expect_equal(names(parsed), "Fazenda: núcleo A")
  expect_equal(parsed[[1]], c("analista", "gestor"))
})

test_that("a group name cannot forge an extra entry", {
  parsed <- parse_roles_header("x%3Aadmin%2Cops:leitor")
  expect_equal(names(parsed), "x:admin,ops")
  expect_equal(parsed[[1]], "leitor")
})

test_that("malformed role entries are skipped", {
  parsed <- parse_roles_header("ok:a,,nocolon,too:many:colons,:r,g:")
  expect_equal(parsed, list(ok = "a"))
  expect_equal(length(parse_roles_header(NULL)), 0)
})
