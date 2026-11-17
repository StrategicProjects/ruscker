"""Sign people in to a backend through a Ruscker portal.

Two calls::

    sso = RusckerSSO(issuer, client_id, client_secret, redirect_uri)
    request = sso.start_sign_in()                       # redirect to request.url,
                                                        # store request.pending
    identity = sso.finish_sign_in(callback_query, pending)
"""

from .client import DEFAULT_SCOPES, PendingSignIn, RusckerSSO, SignInRequest
from .errors import (
    InvalidIDTokenError,
    IssuerMismatchError,
    ProviderError,
    SignInError,
    StateMismatchError,
    TokenExchangeError,
)
from .identity import Identity

__all__ = [
    "DEFAULT_SCOPES",
    "Identity",
    "InvalidIDTokenError",
    "IssuerMismatchError",
    "PendingSignIn",
    "ProviderError",
    "RusckerSSO",
    "SignInError",
    "SignInRequest",
    "StateMismatchError",
    "TokenExchangeError",
]
