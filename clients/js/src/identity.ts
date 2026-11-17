/**
 * Who signed in, read from a verified ID token.
 *
 * Store `sub` as the person's key: a username can change hands after an
 * account is deleted, and `email` is neither verified nor unique.
 */
export class Identity {
  /** The person's permanent id on the portal. **The key to store.** */
  readonly sub: string;
  /** `preferred_username` (needs the `profile` scope). */
  readonly username?: string;
  /** Full name, or the username when the portal has none (`profile` scope). */
  readonly name?: string;
  /** May be absent: not every account has one. */
  readonly email?: string;
  /** Department; may be absent. */
  readonly setor?: string;
  /** Only the groups that give access to this system when it is restricted. */
  readonly groups: readonly string[];
  /** Group → roles held in it; `{}` when none. */
  readonly groupRoles: Readonly<Record<string, readonly string[]>>;
  /** Every claim of the ID token, untouched. */
  readonly claims: Readonly<Record<string, unknown>>;

  constructor(claims: Record<string, unknown>) {
    this.claims = claims;
    this.sub = String(claims.sub);
    this.username = str(claims.preferred_username);
    this.name = str(claims.name);
    this.email = str(claims.email);
    this.setor = str(claims.setor);
    this.groups = strings(claims.groups);
    const roles: Record<string, readonly string[]> = {};
    const raw = claims.group_roles;
    if (raw && typeof raw === "object" && !Array.isArray(raw)) {
      for (const [group, list] of Object.entries(raw)) roles[group] = strings(list);
    }
    this.groupRoles = roles;
  }

  hasGroup(group: string): boolean {
    return this.groups.includes(group);
  }

  hasRole(group: string, role: string): boolean {
    return this.roles(group).includes(role);
  }

  /** The roles held in `group`; `[]` when none. */
  roles(group: string): readonly string[] {
    return Object.hasOwn(this.groupRoles, group) ? (this.groupRoles[group] ?? []) : [];
  }
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function strings(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
}
