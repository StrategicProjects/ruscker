"""Unit tests: no network. A fake provider answers through httpx.MockTransport,
and its ID tokens are signed with a key generated here."""

from __future__ import annotations

import base64
import hashlib
import json
import time
from typing import Any
from urllib.parse import parse_qs, urlsplit

import httpx
import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import ec, rsa
from jwt.algorithms import ECAlgorithm

from ruscker_sso import (
    Identity,
    InvalidIDTokenError,
    IssuerMismatchError,
    PendingSignIn,
    ProviderError,
    RusckerSSO,
    SignInError,
    StateMismatchError,
    TokenExchangeError,
)

ISSUER = "https://portal.example.org"
CLIENT_ID = "my-system"
SECRET = "s3cret:with%odd+chars"
REDIRECT = "https://app.example.org/auth/callback"


def _jwk(private_key: ec.EllipticCurvePrivateKey, kid: str) -> dict[str, Any]:
    jwk = ECAlgorithm.to_jwk(private_key.public_key(), as_dict=True)
    jwk.update({"kid": kid, "alg": "ES256", "use": "sig"})
    return jwk


class FakeProvider:
    """Just enough of the provider: discovery, JWKS and token endpoints."""

    def __init__(self) -> None:
        self.key = ec.generate_private_key(ec.SECP256R1())
        self.kid = "k1"
        self.published = [_jwk(self.key, self.kid)]
        self.discovery_issuer = ISSUER
        self.requests: list[httpx.Request] = []
        self.jwks_fetches = 0
        self.token_status = 200
        self.token_body: dict[str, Any] | None = None
        self.claims: dict[str, Any] = {}
        self.nonce = ""

    def sign(self, claims: dict[str, Any], *, key: Any = None, kid: str | None = None,
             alg: str = "ES256") -> str:
        return jwt.encode(claims, key or self.key, algorithm=alg,
                          headers={"kid": kid or self.kid})

    def default_claims(self) -> dict[str, Any]:
        now = int(time.time())
        return {
            "iss": ISSUER, "aud": CLIENT_ID, "sub": "5966f47e", "iat": now, "exp": now + 300,
            "nonce": self.nonce, "preferred_username": "ana", "name": "Ana Maria de Souza",
            "email": "ana@example.org", "setor": "GAPE", "groups": ["fiscal"],
            "group_roles": {"fiscal": ["analista", "gestor"]},
        }

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        path = request.url.path
        if path == "/.well-known/openid-configuration":
            return httpx.Response(200, json={
                "issuer": self.discovery_issuer,
                "authorization_endpoint": f"{ISSUER}/oauth/authorize",
                "token_endpoint": f"{ISSUER}/oauth/token",
                "jwks_uri": f"{ISSUER}/oauth/jwks",
            })
        if path == "/oauth/jwks":
            self.jwks_fetches += 1
            return httpx.Response(200, json={"keys": self.published})
        if path == "/oauth/token":
            if self.token_body is not None:
                return httpx.Response(self.token_status, json=self.token_body)
            token = self.sign({**self.default_claims(), **self.claims})
            return httpx.Response(200, json={"access_token": "at", "token_type": "Bearer",
                                             "expires_in": 300, "id_token": token})
        return httpx.Response(404)


@pytest.fixture
def provider() -> FakeProvider:
    return FakeProvider()


@pytest.fixture
def sso(provider: FakeProvider) -> RusckerSSO:
    client = httpx.Client(transport=httpx.MockTransport(provider.handler))
    return RusckerSSO(ISSUER, CLIENT_ID, SECRET, REDIRECT, http_client=client)


def _start(sso: RusckerSSO, provider: FakeProvider) -> tuple[PendingSignIn, dict[str, str]]:
    request = sso.start_sign_in()
    provider.nonce = request.pending.nonce
    query = {k: v[0] for k, v in parse_qs(urlsplit(request.url).query).items()}
    return request.pending, query


def _callback(pending: PendingSignIn, **extra: str) -> dict[str, str]:
    return {"code": "the-code", "state": pending.state, "iss": ISSUER, **extra}


# ---- starting ---------------------------------------------------------------

def test_authorization_url(sso: RusckerSSO, provider: FakeProvider) -> None:
    request = sso.start_sign_in()
    parts = urlsplit(request.url)
    assert f"{parts.scheme}://{parts.netloc}{parts.path}" == f"{ISSUER}/oauth/authorize"
    q = parse_qs(parts.query)
    assert all(len(v) == 1 for v in q.values())
    assert q["response_type"] == ["code"]
    assert q["client_id"] == [CLIENT_ID]
    assert q["redirect_uri"] == [REDIRECT]
    assert q["scope"] == ["openid profile email groups roles setor"]
    assert q["code_challenge_method"] == ["S256"]
    assert q["state"] == [request.pending.state]
    assert q["nonce"] == [request.pending.nonce]
    expected = base64.urlsafe_b64encode(
        hashlib.sha256(request.pending.code_verifier.encode()).digest()
    ).rstrip(b"=").decode()
    assert q["code_challenge"] == [expected]
    assert len(expected) == 43
    assert "prompt" not in q


def test_random_values_are_long_distinct_and_pkce_safe(sso: RusckerSSO) -> None:
    seen: set[str] = set()
    allowed = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
    for _ in range(20):
        p = sso.start_sign_in().pending
        for value in (p.state, p.nonce, p.code_verifier):
            assert len(value) >= 43  # 32 random bytes, base64url
            assert set(value) <= allowed
            assert value not in seen
            seen.add(value)


def test_scopes_and_prompt(provider: FakeProvider) -> None:
    client = httpx.Client(transport=httpx.MockTransport(provider.handler))
    sso = RusckerSSO(ISSUER, CLIENT_ID, SECRET, REDIRECT, scopes=["profile"], http_client=client)
    q = parse_qs(urlsplit(sso.start_sign_in(prompt="none").url).query)
    assert q["scope"] == ["openid profile"]
    assert q["prompt"] == ["none"]


def test_discovery_is_cached(sso: RusckerSSO, provider: FakeProvider) -> None:
    sso.start_sign_in()
    sso.start_sign_in()
    assert len(provider.requests) == 1


def test_discovery_issuer_must_match(sso: RusckerSSO, provider: FakeProvider) -> None:
    provider.discovery_issuer = "https://elsewhere.example.org"
    with pytest.raises(IssuerMismatchError) as e:
        sso.start_sign_in()
    assert e.value.where == "the discovery document"


def test_pending_round_trips_through_json() -> None:
    p = PendingSignIn("s", "n", "v")
    assert PendingSignIn.from_json(p.to_json()) == p
    for bad in ["", "{}", "[]", '{"state":"s","nonce":"n"}', '{"state":1,"nonce":"n","code_verifier":"v"}']:
        with pytest.raises(SignInError):
            PendingSignIn.from_json(bad)


def test_repr_hides_the_secret(sso: RusckerSSO) -> None:
    assert SECRET not in repr(sso)


# ---- finishing --------------------------------------------------------------

def test_full_sign_in(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    identity = sso.finish_sign_in(_callback(pending), pending)
    assert identity.sub == "5966f47e"
    assert identity.username == "ana"
    assert identity.name == "Ana Maria de Souza"
    assert identity.email == "ana@example.org"
    assert identity.setor == "GAPE"
    assert identity.groups == ["fiscal"]
    assert identity.group_roles == {"fiscal": ["analista", "gestor"]}
    assert identity.claims["aud"] == CLIENT_ID

    token_request = next(r for r in provider.requests if r.url.path == "/oauth/token")
    form = parse_qs(token_request.content.decode())
    assert form == {"grant_type": ["authorization_code"], "code": ["the-code"],
                    "redirect_uri": [REDIRECT], "code_verifier": [pending.code_verifier]}
    scheme, value = token_request.headers["authorization"].split(" ", 1)
    assert scheme == "Basic"
    # Form-encoded before base64 (RFC 6749 §2.3.1).
    assert base64.b64decode(value).decode() == "my-system:s3cret%3Awith%25odd%2Bchars"


def test_callback_as_string_or_url(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    url = f"{REDIRECT}?code=the-code&state={pending.state}"
    assert sso.finish_sign_in(url, pending).sub == "5966f47e"
    assert sso.finish_sign_in(f"code=the-code&state={pending.state}", pending).sub == "5966f47e"


def test_error_callback(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    before = len(provider.requests)
    with pytest.raises(ProviderError) as e:
        sso.finish_sign_in({"error": "access_denied", "state": pending.state}, pending)
    assert e.value.code == "access_denied"
    assert len(provider.requests) == before


def test_state_mismatch_makes_no_request(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    before = len(provider.requests)
    for state in [pending.state + "x", "", None]:
        cb = _callback(pending)
        if state is None:
            del cb["state"]
        else:
            cb["state"] = state
        with pytest.raises(StateMismatchError):
            sso.finish_sign_in(cb, pending)
    assert len(provider.requests) == before


def test_issuer_mismatch_in_callback(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    before = len(provider.requests)
    with pytest.raises(IssuerMismatchError) as e:
        sso.finish_sign_in(_callback(pending, iss="https://evil.example.org"), pending)
    assert e.value.where == "the callback"
    assert len(provider.requests) == before


def test_iss_absent_is_accepted(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    cb = _callback(pending)
    del cb["iss"]
    assert sso.finish_sign_in(cb, pending).username == "ana"


def test_repeated_parameter_is_refused(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    with pytest.raises(SignInError):
        sso.finish_sign_in(f"code=a&code=b&state={pending.state}", pending)


def test_missing_code(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    with pytest.raises(SignInError):
        sso.finish_sign_in({"state": pending.state}, pending)


def test_token_exchange_refused(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    provider.token_status = 400
    provider.token_body = {"error": "invalid_grant", "error_description": "code already used"}
    with pytest.raises(TokenExchangeError) as e:
        sso.finish_sign_in(_callback(pending), pending)
    assert (e.value.status, e.value.error) == (400, "invalid_grant")


@pytest.mark.parametrize("claims, reason", [
    ({"aud": "another-system"}, "audience"),
    ({"iss": "https://evil.example.org"}, "issuer"),
    ({"exp": int(time.time()) - 3600}, "expired"),
    ({"nonce": "not-the-nonce"}, "nonce"),
    ({"nonce": None}, "nonce"),
    ({"groups": "fiscal"}, "claims"),
])
def test_id_token_claim_checks(sso: RusckerSSO, provider: FakeProvider,
                               claims: dict[str, Any], reason: str) -> None:
    pending, _ = _start(sso, provider)
    provider.claims = claims
    with pytest.raises(InvalidIDTokenError) as e:
        sso.finish_sign_in(_callback(pending), pending)
    assert e.value.reason == reason


def _token_response(provider: FakeProvider, token: str) -> None:
    provider.token_body = {"access_token": "at", "token_type": "Bearer", "id_token": token}


def test_signature_by_another_key_is_refused(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    intruder = ec.generate_private_key(ec.SECP256R1())
    _token_response(provider, provider.sign(provider.default_claims(), key=intruder))
    with pytest.raises(InvalidIDTokenError) as e:
        sso.finish_sign_in(_callback(pending), pending)
    assert e.value.reason == "signature"


def test_only_es256_is_accepted(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    _token_response(provider, provider.sign(provider.default_claims(), key=rsa_key, alg="RS256"))
    with pytest.raises(InvalidIDTokenError) as e:
        sso.finish_sign_in(_callback(pending), pending)
    assert e.value.reason == "algorithm"


def test_unsigned_token_is_refused(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    header = base64.urlsafe_b64encode(json.dumps({"alg": "none", "kid": "k1"}).encode()).rstrip(b"=")
    body = base64.urlsafe_b64encode(json.dumps(provider.default_claims()).encode()).rstrip(b"=")
    _token_response(provider, f"{header.decode()}.{body.decode()}.")
    with pytest.raises(InvalidIDTokenError):
        sso.finish_sign_in(_callback(pending), pending)


def test_unknown_kid_refreshes_the_key_set_once(sso: RusckerSSO, provider: FakeProvider) -> None:
    pending, _ = _start(sso, provider)
    sso.finish_sign_in(_callback(pending), pending)
    assert provider.jwks_fetches == 1

    # The provider rotates: a new key, published, signs the next token.
    new_key = ec.generate_private_key(ec.SECP256R1())
    provider.published = [_jwk(new_key, "k2")]
    pending, _ = _start(sso, provider)
    _token_response(provider, provider.sign(provider.default_claims(), key=new_key, kid="k2"))
    assert sso.finish_sign_in(_callback(pending), pending).sub == "5966f47e"
    assert provider.jwks_fetches == 2

    # A kid that is nowhere: one refresh, then refused.
    _token_response(provider, provider.sign(provider.default_claims(), kid="k9"))
    with pytest.raises(InvalidIDTokenError) as e:
        sso.finish_sign_in(_callback(pending), pending)
    assert e.value.reason == "unknown_key"
    assert provider.jwks_fetches == 3


# ---- identity -----------------------------------------------------------------

def test_identity_helpers() -> None:
    identity = Identity.from_claims({
        "sub": "x", "groups": ["fiscal"], "group_roles": {"fiscal": ["analista", "gestor"]},
    })
    assert identity.has_group("fiscal") and not identity.has_group("outro")
    assert identity.has_role("fiscal", "gestor")
    assert not identity.has_role("fiscal", "leitor")
    assert not identity.has_role("outro", "gestor")
    assert identity.roles("fiscal") == ["analista", "gestor"]
    assert identity.roles("outro") == []


def test_identity_missing_optional_claims() -> None:
    identity = Identity.from_claims({"sub": "x", "iss": ISSUER})
    assert identity.username is None and identity.name is None
    assert identity.email is None and identity.setor is None
    assert identity.groups == [] and identity.group_roles == {}
    assert identity.claims == {"sub": "x", "iss": ISSUER}


def test_identity_refuses_odd_shapes() -> None:
    for claims in [{}, {"sub": 3}, {"sub": "x", "email": 1},
                   {"sub": "x", "group_roles": {"g": "r"}}, {"sub": "x", "groups": [1]}]:
        with pytest.raises(InvalidIDTokenError):
            Identity.from_claims(claims)

