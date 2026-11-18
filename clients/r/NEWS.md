# rusckersso 0.1.0

First release.

* `hosted_identity()` and `hosted_identity_plumber()` read the identity
  a Ruscker portal passes to the applications it hosts.
* `sso_client()`, `sso_start()` and `sso_finish()` sign people in to an
  application running outside the portal, through the portal's OpenID
  Connect provider.
* `has_group()`, `has_role()` and `roles()` answer the usual questions
  about an identity.
