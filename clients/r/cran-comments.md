## Submission

This is a new submission.

## R CMD check results

0 errors | 0 warnings | 1 note

* checking CRAN incoming feasibility ... NOTE
  Maintainer: 'André Leite <leite@castlab.org>'
  New submission

## Notes for the reviewer

* The package signs people in to a web application through an OpenID
  Connect provider. `sso_start()` and `sso_finish()` need a live
  provider (and, for `sso_finish()`, the callback of a real sign-in),
  so their examples are wrapped in `\dontrun{}`. Every other exported
  function has runnable examples.
* The tests that talk to a provider are skipped unless the environment
  variable `RUSCKER_ISSUER` is set, so the checks on CRAN run offline.
  The remaining tests verify the signature and claim checks against
  tokens generated inside the test, with no network access.
* No cryptography is implemented in the package: signatures are
  verified by 'jose', random values come from 'openssl'.

## Downstream dependencies

None: this is a new package.
