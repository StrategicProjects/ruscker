"""The sign-in client: start a sign-in, finish it, get an :class:`Identity`.

The flow is OpenID Connect's authorization code flow with PKCE, which is
the only one the provider accepts:

1. :meth:`RusckerSSO.start_sign_in` builds the URL to send the browser to,
   plus three random values (``state``, ``nonce``, PKCE ``code_verifier``)
   that the application keeps until the person comes back.
2. :meth:`RusckerSSO.finish_sign_in` takes the callback's query and those
   values, redeems the code on the server side and verifies the ID token.

All the cryptography (ES256 signature checks, JWKS parsing) is PyJWT's.
This module only wires the steps together in the right order.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import secrets
import threading
from collections.abc import Mapping, Sequence
from dataclasses import asdict, dataclass, field
from typing import Any
from urllib.parse import parse_qs, quote_plus, urlencode

import httpx
import jwt

from .errors import (
    InvalidIDTokenError,
    IssuerMismatchError,
    ProviderError,
    SignInError,
    StateMismatchError,
    TokenExchangeError,
)
from .identity import Identity

DEFAULT_SCOPES = ("openid", "profile", "email", "groups", "roles", "setor")

# The provider signs ID tokens with ES256 and nothing else. The list is fixed
# here on purpose: reading the algorithm from the token itself would let a
# forged token pick a weaker one.
_ALGORITHMS = ["ES256"]


@dataclass(frozen=True)
class PendingSignIn:
    """The values to keep between :meth:`~RusckerSSO.start_sign_in` and the callback.

    Keep them where only this person's browser session can reach them: a
    server-side session, or a short-lived signed ``HttpOnly`` cookie. They are
    secrets for a few minutes: anyone holding them and the callback's code
    could finish the sign-in.
    """

    # repr=False: these are secrets for a few minutes and must not end up
    # in a log line or a traceback.
    state: str = field(repr=False)
    nonce: str = field(repr=False)
    code_verifier: str = field(repr=False)

    def to_json(self) -> str:
        """Serialise to a short JSON string, e.g. for a session or a signed cookie."""
        return json.dumps(asdict(self), separators=(",", ":"))

    @classmethod
    def from_json(cls, data: str | bytes) -> PendingSignIn:
        """Read back what :meth:`to_json` produced."""
        try:
            raw = json.loads(data)
            values = {k: raw[k] for k in ("state", "nonce", "code_verifier")}
        except (ValueError, TypeError, KeyError) as exc:
            raise SignInError("the stored sign-in values cannot be read") from exc
        if not all(isinstance(v, str) and v for v in values.values()):
            raise SignInError("the stored sign-in values cannot be read")
        return cls(**values)


@dataclass(frozen=True)
class SignInRequest:
    """What :meth:`~RusckerSSO.start_sign_in` returns.

    Redirect the browser to ``url``; store ``pending`` (``pending.to_json()``
    if the store wants a string).
    """

    url: str
    pending: PendingSignIn


def _random_token() -> str:
    # 32 bytes from the OS's cryptographic source, base64url without padding:
    # 43 characters, all in the set PKCE allows for a verifier.
    return secrets.token_urlsafe(32)


def _code_challenge(verifier: str) -> str:
    # PKCE S256 (RFC 7636): BASE64URL(SHA-256(verifier)) without padding.
    # This is an encoding step, not a cryptographic design of ours.
    digest = hashlib.sha256(verifier.encode("ascii")).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")


def _same(a: str, b: str) -> bool:
    # Constant-time comparison, so the timing of a refusal says nothing about
    # how much of a guessed value was right.
    # An empty value never matches: a lost or hand-built pending value must
    # not "equal" an absent parameter.
    if not a or not b:
        return False
    return hmac.compare_digest(a.encode("utf-8"), b.encode("utf-8"))


def _parse_callback(query: str | Mapping[str, Any]) -> dict[str, str]:
    """Turn a callback's query into ``{name: value}``.

    Accepts a raw query string (with or without ``?``), a full callback URL,
    or a mapping (a framework's query object, or ``parse_qs`` output). A
    parameter given twice is refused: there is no safe way to pick one.
    Prefer passing the raw query string or URL: a framework's mapping has
    usually collapsed repeated parameters already, so they cannot be seen.
    """
    if isinstance(query, str):
        text = query.split("?", 1)[1] if "?" in query else query
        text = text.split("#", 1)[0]
        items: Mapping[str, Any] = parse_qs(text, keep_blank_values=True)
    else:
        items = query
    result: dict[str, str] = {}
    for key, value in items.items():
        if isinstance(value, (list, tuple)):
            if len(value) != 1:
                raise SignInError(f"the callback repeats the parameter {key!r}")
            value = value[0]
        if not isinstance(value, str):
            raise SignInError(f"the callback parameter {key!r} is not text")
        result[key] = value
    return result


class RusckerSSO:
    """A system registered on a Ruscker portal, ready to sign people in.

    Create one per application (at start-up) and share it: it caches the
    discovery document and the provider's keys, and is safe to use from
    several threads.

    ``issuer`` must be written exactly as the portal's discovery document
    states it (including a subpath when the portal is mounted under one).
    ``http_client`` lets the application supply its own ``httpx.Client``
    (proxies, custom CA bundle, or a mock in tests). ``leeway`` is the clock
    skew, in seconds, tolerated on ``exp``/``iat``.
    """

    def __init__(
        self,
        issuer: str,
        client_id: str,
        client_secret: str,
        redirect_uri: str,
        scopes: str | Sequence[str] = DEFAULT_SCOPES,
        *,
        http_client: httpx.Client | None = None,
        timeout: float = 10.0,
        leeway: float = 60.0,
    ) -> None:
        self.issuer = issuer
        self.client_id = client_id
        self._client_secret = client_secret
        self.redirect_uri = redirect_uri
        scope_list = scopes.split() if isinstance(scopes, str) else list(scopes)
        if "openid" not in scope_list:
            scope_list.insert(0, "openid")
        self.scope = " ".join(scope_list)
        self._http = http_client or httpx.Client(timeout=timeout)
        self._leeway = leeway
        self._lock = threading.Lock()
        self._discovery: dict[str, Any] | None = None
        self._jwks: jwt.PyJWKSet | None = None

    def __repr__(self) -> str:  # never print the secret
        return f"RusckerSSO(issuer={self.issuer!r}, client_id={self.client_id!r})"

    # ---- starting ---------------------------------------------------------

    def start_sign_in(self, *, prompt: str | None = None) -> SignInRequest:
        """Build the authorization URL and the values to keep.

        ``prompt="none"`` asks the provider to answer with an error instead
        of showing a sign-in page when the person would have to interact.
        """
        pending = PendingSignIn(
            state=_random_token(), nonce=_random_token(), code_verifier=_random_token()
        )
        params = {
            "response_type": "code",
            "client_id": self.client_id,
            "redirect_uri": self.redirect_uri,
            "scope": self.scope,
            "state": pending.state,
            "nonce": pending.nonce,
            "code_challenge": _code_challenge(pending.code_verifier),
            "code_challenge_method": "S256",
        }
        if prompt is not None:
            params["prompt"] = prompt
        endpoint = self._discover()["authorization_endpoint"]
        separator = "&" if "?" in endpoint else "?"
        return SignInRequest(url=endpoint + separator + urlencode(params), pending=pending)

    # ---- finishing --------------------------------------------------------

    def finish_sign_in(
        self, callback_query: str | Mapping[str, Any], pending: PendingSignIn
    ) -> Identity:
        """Check the callback, redeem the code, verify the ID token.

        The checks run in this order, and the first three need no network,
        so a forged callback never reaches the token endpoint:

        1. an ``error`` parameter → :class:`ProviderError`;
        2. ``state`` differs from the stored one → :class:`StateMismatchError`;
        3. ``iss`` is absent or differs from the issuer → :class:`IssuerMismatchError`;
        4. the code exchange is refused → :class:`TokenExchangeError`;
        5. the ID token does not verify → :class:`InvalidIDTokenError`.

        Discard ``pending`` afterwards whatever the outcome: a code is
        single-use, so a second attempt cannot succeed.
        """
        params = _parse_callback(callback_query)

        if "error" in params:
            raise ProviderError(params["error"], params.get("error_description"))

        if not _same(params.get("state", ""), pending.state):
            raise StateMismatchError("the callback's state is not the one this sign-in started with")

        # The portal always says who it is (RFC 9207). Requiring it means a
        # callback from anywhere else cannot be mistaken for the portal's.
        if params.get("iss") != self.issuer:
            raise IssuerMismatchError(self.issuer, str(params.get("iss")), "the callback")

        code = params.get("code")
        if not code:
            raise SignInError("the callback has neither a code nor an error")

        tokens = self._exchange_code(code, pending.code_verifier)
        id_token = tokens.get("id_token")
        if not isinstance(id_token, str):
            raise InvalidIDTokenError("malformed", "the token response has no id_token")
        return self._verify_id_token(id_token, pending.nonce)

    # ---- steps ------------------------------------------------------------

    def _get_json(self, url: str, what: str) -> dict[str, Any]:
        try:
            response = self._http.get(url, headers={"Accept": "application/json"})
            response.raise_for_status()
            data = response.json()
        except (httpx.HTTPError, ValueError) as exc:
            raise SignInError(f"could not read the {what} at {url}: {exc}") from exc
        if not isinstance(data, dict):
            raise SignInError(f"the {what} at {url} is not a JSON object")
        return data

    def _discover(self) -> dict[str, Any]:
        """Fetch ``{issuer}/.well-known/openid-configuration`` once and keep it."""
        with self._lock:
            if self._discovery is None:
                url = self.issuer.rstrip("/") + "/.well-known/openid-configuration"
                doc = self._get_json(url, "discovery document")
                if doc.get("issuer") != self.issuer:
                    raise IssuerMismatchError(self.issuer, str(doc.get("issuer")), "the discovery document")
                for key in ("authorization_endpoint", "token_endpoint", "jwks_uri"):
                    if not isinstance(doc.get(key), str):
                        raise SignInError(f"the discovery document has no {key}")
                self._discovery = doc
            return self._discovery

    def _exchange_code(self, code: str, code_verifier: str) -> dict[str, Any]:
        """Redeem the code at the token endpoint (client_secret_basic)."""
        token_endpoint = self._discover()["token_endpoint"]
        # RFC 6749 §2.3.1: id and secret are form-encoded before going into
        # the Basic header, so a ':' or '%' in them survives the trip.
        basic = base64.b64encode(
            f"{quote_plus(self.client_id)}:{quote_plus(self._client_secret)}".encode()
        ).decode("ascii")
        try:
            response = self._http.post(
                token_endpoint,
                data={
                    "grant_type": "authorization_code",
                    "code": code,
                    "redirect_uri": self.redirect_uri,
                    "code_verifier": code_verifier,
                },
                headers={"Authorization": f"Basic {basic}", "Accept": "application/json"},
            )
        except httpx.HTTPError as exc:
            # Not retried: the provider may already have spent the code.
            raise SignInError(f"could not reach the token endpoint: {exc}") from exc

        try:
            body = response.json()
        except ValueError:
            body = None
        if response.status_code != 200:
            error = body.get("error") if isinstance(body, dict) else None
            description = body.get("error_description") if isinstance(body, dict) else None
            raise TokenExchangeError(response.status_code, error, description)
        if not isinstance(body, dict):
            raise SignInError("the token response is not a JSON object")
        return body

    def _signing_key(self, kid: str) -> Any:
        """The provider's public key named ``kid``, refreshing the set once if unknown.

        The provider can rotate keys; a token signed by a key we have not
        seen yet triggers exactly one new fetch of the JWKS.
        """
        jwks_uri = self._discover()["jwks_uri"]
        with self._lock:
            for attempt in range(2):
                if self._jwks is None or attempt == 1:
                    data = self._get_json(jwks_uri, "key set")
                    try:
                        self._jwks = jwt.PyJWKSet.from_dict(data)
                    except jwt.PyJWKSetError as exc:
                        raise SignInError(f"the provider's key set is unusable: {exc}") from exc
                try:
                    return self._jwks[kid].key
                except KeyError:
                    continue
        raise InvalidIDTokenError("unknown_key", f"no key {kid!r} in the provider's key set")

    def _verify_id_token(self, id_token: str, nonce: str) -> Identity:
        """Verify signature (ES256 only), ``iss``, ``aud``, ``exp`` and ``nonce``."""
        try:
            # Only the key id is read before verification; never the algorithm.
            kid = jwt.get_unverified_header(id_token).get("kid")
        except jwt.DecodeError as exc:
            raise InvalidIDTokenError("malformed", str(exc)) from exc
        if not isinstance(kid, str):
            raise InvalidIDTokenError("unknown_key", "the token names no key id")

        key = self._signing_key(kid)
        try:
            claims = jwt.decode(
                id_token,
                key,
                algorithms=_ALGORITHMS,
                audience=self.client_id,
                issuer=self.issuer,
                leeway=self._leeway,
                options={"require": ["iss", "aud", "sub", "exp", "iat"]},
            )
        except jwt.InvalidAlgorithmError as exc:
            raise InvalidIDTokenError("algorithm", str(exc)) from exc
        except jwt.InvalidSignatureError as exc:
            raise InvalidIDTokenError("signature", str(exc)) from exc
        except jwt.ExpiredSignatureError as exc:
            raise InvalidIDTokenError("expired", str(exc)) from exc
        except jwt.ImmatureSignatureError as exc:
            raise InvalidIDTokenError("not_yet_valid", str(exc)) from exc
        except jwt.InvalidAudienceError as exc:
            raise InvalidIDTokenError("audience", str(exc)) from exc
        except jwt.InvalidIssuerError as exc:
            raise InvalidIDTokenError("issuer", str(exc)) from exc
        except jwt.MissingRequiredClaimError as exc:
            raise InvalidIDTokenError("missing_claim", str(exc)) from exc
        except jwt.InvalidTokenError as exc:
            raise InvalidIDTokenError("malformed", str(exc)) from exc

        # PyJWT does not know about nonce; it ties this token to this sign-in.
        token_nonce = claims.get("nonce")
        if not isinstance(token_nonce, str) or not _same(token_nonce, nonce):
            raise InvalidIDTokenError("nonce", "the token's nonce is not the one this sign-in sent")

        return Identity.from_claims(claims)
