"""The person who signed in, as the verified ID token describes them."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any

from .errors import InvalidIDTokenError


@dataclass(frozen=True)
class Identity:
    """Who signed in.

    Store ``sub``: it is the person's permanent id on the portal. A username
    can be reused after an account is deleted, and ``email`` is neither
    verified nor unique.

    Fields that depend on a scope (or on the account having the data) are
    ``None`` when absent; ``groups`` and ``group_roles`` are empty then.
    """

    sub: str
    username: str | None
    name: str | None
    email: str | None
    setor: str | None
    groups: list[str]
    group_roles: dict[str, list[str]]
    #: Every claim of the verified ID token, untouched.
    claims: dict[str, Any] = field(repr=False)

    def has_group(self, group: str) -> bool:
        """True when the person is in ``group`` (as far as this system may know)."""
        return group in self.groups

    def has_role(self, group: str, role: str) -> bool:
        """True when the person holds ``role`` in ``group``."""
        return role in self.group_roles.get(group, [])

    def roles(self, group: str) -> list[str]:
        """The person's roles in ``group``; an empty list when none."""
        return list(self.group_roles.get(group, []))

    @classmethod
    def from_claims(cls, claims: Mapping[str, Any]) -> Identity:
        """Build an identity from already-verified claims.

        Types are checked here rather than trusted: a claim of the wrong
        shape means the token is not what this package understands, and
        it is better to refuse it than to hand the application a surprise.
        """
        sub = claims.get("sub")
        if not isinstance(sub, str) or not sub:
            raise InvalidIDTokenError("claims", "sub is missing or not a string")

        def optional_str(name: str) -> str | None:
            value = claims.get(name)
            if value is None:
                return None
            if not isinstance(value, str):
                raise InvalidIDTokenError("claims", f"{name} is not a string")
            return value

        groups = claims.get("groups", [])
        if not isinstance(groups, list) or not all(isinstance(g, str) for g in groups):
            raise InvalidIDTokenError("claims", "groups is not a list of strings")

        group_roles = claims.get("group_roles", {})
        if not isinstance(group_roles, dict) or not all(
            isinstance(k, str) and isinstance(v, list) and all(isinstance(r, str) for r in v)
            for k, v in group_roles.items()
        ):
            raise InvalidIDTokenError("claims", "group_roles is not a map of group to list of roles")

        return cls(
            sub=sub,
            username=optional_str("preferred_username"),
            name=optional_str("name"),
            email=optional_str("email"),
            setor=optional_str("setor"),
            groups=list(groups),
            group_roles={k: list(v) for k, v in group_roles.items()},
            claims=dict(claims),
        )
