# rusckersso 0.2.0

* `sso_sign_out_url()` builds the address of the portal's sign-out page
  (OpenID Connect RP-Initiated Logout), from the discovery document's
  `end_session_endpoint`. A portal that does not announce one is reported
  as a `rusckersso_http_error`.
* `sso_password_url()` returns the address of the portal's "change your
  password" page for this client. No network call.
* `sso_start()` gains `prompt`: `prompt = "none"` checks quietly whether
  the person is still signed in to the portal. Without it the URL is
  unchanged.

# rusckersso 0.1.0

First release.

* `hosted_identity()` and `hosted_identity_plumber()` read the identity
  a Ruscker portal passes to the applications it hosts.
* `sso_client()`, `sso_start()` and `sso_finish()` sign people in to an
  application running outside the portal, through the portal's OpenID
  Connect provider.
* `has_group()`, `has_role()` and `roles()` answer the usual questions
  about an identity.
