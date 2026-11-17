"""End-to-end against a real provider started by clients/testing/provider.sh.

Skipped unless RUSCKER_ISSUER is set (source the env file it writes).
`authorize.sh` plays the signed-in person's browser.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import replace
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx
import pytest

from ruscker_sso import (
    InvalidIDTokenError,
    PendingSignIn,
    RusckerSSO,
    StateMismatchError,
    TokenExchangeError,
)

pytestmark = pytest.mark.skipif(
    not os.environ.get("RUSCKER_ISSUER"), reason="RUSCKER_ISSUER not set (no test provider)"
)

AUTHORIZE = Path(__file__).resolve().parents[2] / "testing" / "authorize.sh"


def _client(**overrides: str) -> RusckerSSO:
    settings = {
        "issuer": os.environ["RUSCKER_ISSUER"],
        "client_id": os.environ["RUSCKER_CLIENT_ID"],
        "client_secret": os.environ["RUSCKER_CLIENT_SECRET"],
        "redirect_uri": os.environ["RUSCKER_REDIRECT_URI"],
        **overrides,
    }
    return RusckerSSO(**settings)


def _authorize(sso: RusckerSSO) -> tuple[str, PendingSignIn]:
    """Start a sign-in and let the test person's browser follow it."""
    request = sso.start_sign_in()
    callback = subprocess.run(
        [str(AUTHORIZE), request.url], check=True, capture_output=True, text=True
    ).stdout.strip()
    assert callback.startswith(os.environ["RUSCKER_REDIRECT_URI"] + "?"), callback
    return callback, request.pending


def test_full_sign_in_returns_the_identity() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    person = sso.finish_sign_in(callback, pending)
    assert person.sub and person.sub != "ana"
    assert person.username == "ana"
    assert person.name == "Ana Maria de Souza"
    assert person.email == "ana@example.org"
    assert person.setor == "GAPE"
    assert person.groups == ["fiscal"]
    assert person.group_roles == {"fiscal": ["analista", "gestor"]}
    assert person.has_role("fiscal", "gestor") and not person.has_group("outro")
    assert person.claims["aud"] == os.environ["RUSCKER_CLIENT_ID"]


def test_tampered_state_is_refused_before_any_network_call() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    seen: list[httpx.Request] = []
    watched = _client()
    watched._http = httpx.Client(event_hooks={"request": [seen.append]})
    with pytest.raises(StateMismatchError):
        watched.finish_sign_in(callback, replace(pending, state=pending.state + "x"))
    assert seen == []
    # The code was not spent: the genuine pending values still work.
    assert sso.finish_sign_in(callback, pending).username == "ana"


def test_replayed_code_is_refused() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    sso.finish_sign_in(callback, pending)
    with pytest.raises(TokenExchangeError) as e:
        sso.finish_sign_in(callback, pending)
    assert e.value.error == "invalid_grant"


def test_wrong_nonce_is_refused() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    with pytest.raises(InvalidIDTokenError) as e:
        sso.finish_sign_in(callback, replace(pending, nonce="another-nonce"))
    assert e.value.reason == "nonce"


def test_wrong_client_secret_is_refused() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    with pytest.raises(TokenExchangeError) as e:
        _client(client_secret="0" * 64).finish_sign_in(callback, pending)
    assert (e.value.status, e.value.error) == (401, "invalid_client")
    # A failed client authentication does not spend the code.
    assert sso.finish_sign_in(callback, pending).username == "ana"


def test_token_for_another_audience_is_refused() -> None:
    sso = _client()
    callback, pending = _authorize(sso)
    code = parse_qs(urlsplit(callback).query)["code"][0]
    id_token = sso._exchange_code(code, pending.code_verifier)["id_token"]
    # The same, genuine token, presented to a system registered as "other".
    with pytest.raises(InvalidIDTokenError) as e:
        _client(client_id="other")._verify_id_token(id_token, pending.nonce)
    assert e.value.reason == "audience"
    # Sanity: the token is fine for its real audience.
    assert sso._verify_id_token(id_token, pending.nonce).username == "ana"
