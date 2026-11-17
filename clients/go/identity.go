package rusckersso

import (
	"encoding/json"
	"errors"
	"slices"
)

// Identity is who signed in. Store Sub as the key for the person: a
// username can be reused after an account is deleted and the e-mail is
// neither verified nor unique.
type Identity struct {
	// Sub is the person's permanent id on the portal.
	Sub string `json:"sub"`
	// Username is preferred_username.
	Username string `json:"username"`
	// Name is the full name, or the username when the portal has none.
	Name string `json:"name"`
	// Email is "" when the account has none (or the scope was not asked).
	Email string `json:"email,omitempty"`
	// Setor is the department; "" when absent.
	Setor string `json:"setor,omitempty"`
	// Groups the person holds that give access to this system (all of
	// their groups when the system is open). Never nil.
	Groups []string `json:"groups"`
	// GroupRoles maps group → roles in it. Never nil; empty when none.
	GroupRoles map[string][]string `json:"group_roles"`
	// Claims are the ID token's claims as decoded JSON, for anything the
	// fields above do not cover.
	Claims map[string]any `json:"-"`
}

// HasGroup reports whether the person is in group.
func (id *Identity) HasGroup(group string) bool {
	return slices.Contains(id.Groups, group)
}

// HasRole reports whether the person holds role in group.
func (id *Identity) HasRole(group, role string) bool {
	return slices.Contains(id.GroupRoles[group], role)
}

// Roles lists the person's roles in group (nil when none).
func (id *Identity) Roles(group string) []string {
	return slices.Clone(id.GroupRoles[group])
}

// identityFromClaims maps the token's claims onto the contract's fields.
// Going through JSON again keeps the typing in one place (the struct tags)
// rather than in hand-written type assertions.
func identityFromClaims(claims map[string]any) (*Identity, error) {
	raw, err := json.Marshal(claims)
	if err != nil {
		return nil, err
	}
	var c struct {
		Sub        string              `json:"sub"`
		Username   string              `json:"preferred_username"`
		Name       string              `json:"name"`
		Email      string              `json:"email"`
		Setor      string              `json:"setor"`
		Groups     []string            `json:"groups"`
		GroupRoles map[string][]string `json:"group_roles"`
	}
	if err := json.Unmarshal(raw, &c); err != nil {
		return nil, err
	}
	id := &Identity{
		Sub: c.Sub, Username: c.Username, Name: c.Name, Email: c.Email, Setor: c.Setor,
		Groups: c.Groups, GroupRoles: c.GroupRoles, Claims: claims,
	}
	// `sub` is the key applications store; an identity without one is
	// not an identity.
	if id.Sub == "" {
		return nil, errors.New("the token has no subject (sub)")
	}
	if id.Groups == nil {
		id.Groups = []string{}
	}
	if id.GroupRoles == nil {
		id.GroupRoles = map[string][]string{}
	}
	return id, nil
}
