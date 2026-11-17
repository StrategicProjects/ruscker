"""Errors raised by a sign-in.

Every failure is a :class:`SignInError`, so a callback route can catch one
type and show "sign-in failed". The subclasses exist so the application can
log, or react to, the specific reason.
"""

from __future__ import annotations


class SignInError(Exception):
    """Base class: the sign-in did not produce an identity.

    Raised directly for problems that are not the person's doing and have no
    finer category: the provider is unreachable, the discovery document or a
    response is malformed, the callback has no ``code``, the stored values
    cannot be read back.
    """


class ProviderError(SignInError):
    """The provider sent the person back with ``error=...`` instead of a code.

    ``code`` is the OAuth error code, e.g. ``access_denied`` (the person is
    not in the system's allowed groups) or ``invalid_request``.
    """

    def __init__(self, code: str, description: str | None = None) -> None:
        self.code = code
        self.description = description
        message = f"the provider refused the sign-in: {code}"
        if description:
            message += f" ({description})"
        super().__init__(message)


class StateMismatchError(SignInError):
    """The callback's ``state`` is not the one stored when the sign-in began.

    Usually a stale or forged callback (or a second tab). Checked before any
    network call, so the code is never redeemed.
    """


class IssuerMismatchError(SignInError):
    """An issuer is not the configured one.

    Either the callback's ``iss`` parameter, or the ``issuer`` in the
    discovery document, differs from the issuer this client was set up with.
    """

    def __init__(self, expected: str, got: str, where: str) -> None:
        self.expected = expected
        self.got = got
        self.where = where
        super().__init__(f"issuer mismatch in {where}: expected {expected!r}, got {got!r}")


class TokenExchangeError(SignInError):
    """The token endpoint refused the code.

    ``error`` is the provider's code: ``invalid_grant`` (code used, expired,
    or redeemed with another verifier / redirect URI), ``invalid_client``
    (wrong secret or disabled client), ``invalid_request``, ...
    ``status`` is the HTTP status. The code is spent; never retry with it.
    """

    def __init__(self, status: int, error: str | None, description: str | None = None) -> None:
        self.status = status
        self.error = error
        self.description = description
        message = f"token exchange refused (HTTP {status}): {error or 'no error code'}"
        if description:
            message += f" ({description})"
        super().__init__(message)


class InvalidIDTokenError(SignInError):
    """The ID token failed verification.

    ``reason`` is a short stable word: ``signature``, ``algorithm``,
    ``audience``, ``issuer``, ``expired``, ``not_yet_valid``, ``nonce``,
    ``missing_claim``, ``unknown_key``, ``malformed`` or ``claims``.
    """

    def __init__(self, reason: str, detail: str | None = None) -> None:
        self.reason = reason
        self.detail = detail
        message = f"ID token rejected: {reason}"
        if detail:
            message += f" ({detail})"
        super().__init__(message)
