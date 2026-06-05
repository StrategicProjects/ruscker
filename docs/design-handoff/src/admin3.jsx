/* global React, window */
// ── Admin: Users · Groups · Audit (multi-group model) ──
const { useState: a3S, useMemo: a3M } = React;
const A3 = window.RKC;

const initials = (n) => n.split(" ").map((w) => w[0]).slice(0, 2).join("").toUpperCase();
const AV_COLORS = ["#447099", "#059487", "#ef476f", "#7c3aed", "#f37726", "#0d76bf"];
const avColor = (s) => AV_COLORS[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % AV_COLORS.length];

function Avatar({ name, size = 30 }) {
  return <span className="rk-avatar" style={{ width: size, height: size, fontSize: size * 0.38, background: `color-mix(in srgb, ${avColor(name)} 18%, var(--surface))`, color: avColor(name) }}>{initials(name)}</span>;
}

const ALL_GROUPS = ["admin", "editor", "viewer"];
const GROUP_COLOR = { admin: "#ef476f", editor: "#06b6d4", viewer: "#26547c" };
const GROUP_LABEL = {
  pt: { admin: "Administrador", editor: "Editor", viewer: "Visualizador" },
  en: { admin: "Administrator", editor: "Editor", viewer: "Viewer" },
};
const ST_LABEL = {
  pt: { active: "ativo", invited: "convidado", suspended: "suspenso" },
  en: { active: "active", invited: "invited", suspended: "suspended" },
};

// Users: each belongs to one or more groups
const SEED_USERS = [
  { id: 1, name: "Ana Ribeiro",   email: "ana@ufpe.br",    groups: ["admin","editor"],   status: "active",    seen: { pt: "há 2 min",  en: "2 min ago" } },
  { id: 2, name: "Carlos Mendes", email: "carlos@ufpe.br", groups: ["editor"],           status: "active",    seen: { pt: "há 1 h",    en: "1 h ago" } },
  { id: 3, name: "Beatriz Lima",  email: "bia@ufpe.br",    groups: ["editor","viewer"],  status: "active",    seen: { pt: "ontem",     en: "yesterday" } },
  { id: 4, name: "Diego Souza",   email: "diego@ufpe.br",  groups: ["viewer"],           status: "invited",   seen: { pt: "—",         en: "—" } },
  { id: 5, name: "Elena Castro",  email: "elena@ext.com",  groups: ["viewer"],           status: "active",    seen: { pt: "há 3 dias", en: "3 days ago" } },
  { id: 6, name: "Felipe Araújo", email: "felipe@ufpe.br", groups: ["editor"],           status: "suspended", seen: { pt: "há 21 dias",en: "21 days ago" } },
];
// Export for portal simulation
window.RUSCKER_USERS = SEED_USERS;

function GroupBadge({ g, lang }) {
  return (
    <span className="group-badge" style={{
      background: `color-mix(in srgb, ${GROUP_COLOR[g]} 16%, transparent)`,
      color: GROUP_COLOR[g],
    }}>{GROUP_LABEL[lang][g]}</span>
  );
}

/* ══════════════════════════════════════ USERS ═══════════════════ */
function AdminUsers({ t, loading, push }) {
  const lang = t._lang;
  const [users, setUsers] = a3S(SEED_USERS);
  const [q, setQ] = a3S("");
  const [stF, setStF] = a3S("all");
  const [editId, setEditId] = a3S(null);

  const shown = users.filter((u) => {
    if (stF !== "all" && u.status !== stF) return false;
    if (q.trim() && !(u.name + " " + u.email).toLowerCase().includes(q.trim().toLowerCase())) return false;
    return true;
  });
  const counts = {
    all: users.length,
    active: users.filter((u) => u.status === "active").length,
    invited: users.filter((u) => u.status === "invited").length,
    suspended: users.filter((u) => u.status === "suspended").length,
  };
  const suspend = (id) => setUsers((l) => l.map((u) => u.id === id ? { ...u, status: u.status === "suspended" ? "active" : "suspended" } : u));
  const toggleGroup = (uid, g) => setUsers((l) => l.map((u) => {
    if (u.id !== uid) return u;
    const gs = u.groups.includes(g) ? u.groups.filter((x) => x !== g) : [...u.groups, g];
    return { ...u, groups: gs.length ? gs : u.groups }; // min 1 group
  }));

  const Chip = ({ k, label }) => <button className={"chip" + (stF === k ? " on-all" : "")} onClick={() => setStF(stF === k ? "all" : k)}>{label} <span className="chip-count">{counts[k]}</span></button>;

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 18 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Usuários" : "Users"}</div>
          <div className="page-sub">{lang === "pt" ? "Um usuário pode pertencer a vários grupos" : "A user can belong to multiple groups"}</div>
        </div>
        <button className="btn btn--primary" onClick={() => push(lang === "pt" ? "Convite enviado por e-mail" : "Invite sent by email", "ti-mail")}><i className="ti ti-user-plus"></i>{lang === "pt" ? "Convidar usuário" : "Invite user"}</button>
      </div>

      <div className="admin-sticky">
        <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
          <div className="search-pill" style={{ maxWidth: 280, flex: "0 1 280px", padding: "8px 14px" }}>
            <i className="ti ti-search"></i>
            <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={lang === "pt" ? "Buscar nome ou e-mail…" : "Search name or email…"} />
          </div>
          <Chip k="active" label={ST_LABEL[lang].active} />
          <Chip k="invited" label={ST_LABEL[lang].invited} />
          <Chip k="suspended" label={ST_LABEL[lang].suspended} />
        </div>
      </div>

      {loading ? <div style={{ marginTop: 12 }}><A3.SkBlock w={"100%"} h={300} r={12} /></div> : (
        <table className="admin-table" style={{ marginTop: 12 }}>
          <thead><tr>
            <th>{lang === "pt" ? "Usuário" : "User"}</th>
            <th style={{ width: 280 }}>{lang === "pt" ? "Grupos (clique ✎ para editar)" : "Groups (click ✎ to edit)"}</th>
            <th style={{ width: 110 }}>{lang === "pt" ? "Estado" : "Status"}</th>
            <th style={{ width: 120 }}>{lang === "pt" ? "Visto" : "Last seen"}</th>
            <th style={{ width: 70, textAlign: "right" }}></th>
          </tr></thead>
          <tbody>
            {shown.map((u) => (
              <React.Fragment key={u.id}>
                <tr style={{ opacity: u.status === "suspended" ? 0.55 : 1 }}>
                  <td><span style={{ display: "inline-flex", alignItems: "center", gap: 10 }}>
                    <Avatar name={u.name} />
                    <span><span style={{ fontWeight: 500, display: "block" }}>{u.name}</span><span className="faint mono" style={{ fontSize: 11 }}>{u.email}</span></span>
                  </span></td>
                  <td>
                    <div style={{ display: "flex", gap: 5, flexWrap: "wrap", alignItems: "center" }}>
                      {u.groups.map((g) => <GroupBadge key={g} g={g} lang={lang} />)}
                      <button className="star-toggle" style={{ width: 22, height: 22 }} title={lang === "pt" ? "Editar grupos" : "Edit groups"} onClick={() => setEditId(editId === u.id ? null : u.id)}>
                        <i className={"ti " + (editId === u.id ? "ti-x" : "ti-pencil")} style={{ fontSize: 12 }}></i>
                      </button>
                    </div>
                  </td>
                  <td><span className={"status-pill status-pill--" + u.status}><span className={"dot " + (u.status === "active" ? "dot-ready" : u.status === "invited" ? "dot-boot" : "dot-off")}></span>{ST_LABEL[lang][u.status]}</span></td>
                  <td className="muted" style={{ fontSize: 11.5 }}>{u.seen[lang]}</td>
                  <td style={{ textAlign: "right" }}>
                    <button className="star-toggle" title={u.status === "suspended" ? (lang === "pt" ? "Reativar" : "Reactivate") : (lang === "pt" ? "Suspender" : "Suspend")} onClick={() => suspend(u.id)}>
                      <i className={"ti " + (u.status === "suspended" ? "ti-user-check" : "ti-user-x")}></i>
                    </button>
                  </td>
                </tr>
                {editId === u.id && (
                  <tr className="fade-in">
                    <td></td>
                    <td colSpan={4}>
                      <div className="group-editor-row">
                        <span className="faint" style={{ fontSize: 12 }}>{lang === "pt" ? "Grupos de" : "Groups for"} <b>{u.name}</b>:</span>
                        {ALL_GROUPS.map((g) => (
                          <button key={g} className={"group-toggle" + (u.groups.includes(g) ? " on" : "")}
                            style={u.groups.includes(g) ? { "--gc": GROUP_COLOR[g], borderColor: GROUP_COLOR[g], background: `color-mix(in srgb, ${GROUP_COLOR[g]} 14%, transparent)`, color: GROUP_COLOR[g] } : null}
                            onClick={() => toggleGroup(u.id, g)}>
                            <i className={"ti " + (u.groups.includes(g) ? "ti-check" : "ti-plus")} style={{ fontSize: 12 }}></i>
                            {GROUP_LABEL[lang][g]}
                          </button>
                        ))}
                        <span className="faint" style={{ fontSize: 11 }}>{lang === "pt" ? "mín. 1" : "min 1"}</span>
                      </div>
                    </td>
                  </tr>
                )}
              </React.Fragment>
            ))}
            {shown.length === 0 && <tr><td colSpan={5} style={{ textAlign: "center", color: "var(--text-faint)", padding: 30, fontSize: 12 }}>{lang === "pt" ? "Nenhum usuário." : "No users."}</td></tr>}
          </tbody>
        </table>
      )}
    </div>
  );
}

/* ══════════════════════════════════════ GROUPS ════════════════════ */
const SEED_GROUPS = [
  { id: "admin",  name: { pt: "Administradores", en: "Administrators" }, desc: { pt: "Acesso total ao admin e a todos os apps.",        en: "Full access to admin and all apps." },            perms: ["manage","deploy","view"], color: "#ef476f" },
  { id: "editor", name: { pt: "Editores",        en: "Editors" },        desc: { pt: "Gerenciam apps e mídia, sem acesso a credenciais.", en: "Manage apps and media, no credential access." }, perms: ["deploy","view"],          color: "#06b6d4" },
  { id: "viewer", name: { pt: "Visualizadores",  en: "Viewers" },        desc: { pt: "Acessam apps restritos do portal.",                 en: "Access restricted portal apps." },               perms: ["view"],                   color: "#26547c" },
];
const PERM_LABEL = {
  pt: { manage: "Gerenciar", deploy: "Deploy", view: "Visualizar" },
  en: { manage: "Manage",    deploy: "Deploy", view: "View" },
};

function AdminGroups({ t, loading, push, appAccess }) {
  const lang = t._lang;
  const [sel, setSel] = a3S(null); // selected group id for detail
  const APPS = window.RK.APPS;

  const groupMembers = (gid) => SEED_USERS.filter((u) => u.groups.includes(gid));
  const groupApps = (gid) => APPS.filter((a) => {
    const ag = appAccess ? (appAccess.get(a.id) || a.accessGroups || []) : (a.accessGroups || []);
    return ag.includes(gid);
  });
  const publicApps = APPS.filter((a) => {
    const ag = appAccess ? (appAccess.get(a.id) || a.accessGroups || []) : (a.accessGroups || []);
    return ag.length === 0;
  });

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 18 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Grupos" : "Groups"}</div>
          <div className="page-sub">{lang === "pt" ? "Papéis de acesso — apps podem pertencer a vários grupos" : "Access roles — apps can belong to multiple groups"}</div>
        </div>
        <button className="btn btn--primary" onClick={() => push(lang === "pt" ? "Novo grupo (em breve)" : "New group (soon)", "ti-plus")}><i className="ti ti-plus"></i>{lang === "pt" ? "Novo grupo" : "New group"}</button>
      </div>

      {loading ? <div className="groups-grid">{Array.from({ length: 3 }).map((_, i) => <A3.SkBlock key={i} w={"100%"} h={220} r={12} />)}</div> : (
        <div className="groups-grid stagger">
          {SEED_GROUPS.map((g, i) => {
            const members = groupMembers(g.id);
            const apps = groupApps(g.id);
            const isOpen = sel === g.id;
            return (
              <div className={"group-card" + (isOpen ? " is-sel" : "")} key={g.id} style={{ animationDelay: Math.min(i, 10) * 30 + "ms" }}>
                <div className="group-card__bar" style={{ background: g.color }}></div>
                <div className="group-card__body">
                  <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between" }}>
                    <div className="group-card__name">{g.name[lang]}</div>
                    <button className="star-toggle" style={{ marginTop: -2 }} onClick={() => setSel(isOpen ? null : g.id)}><i className={"ti " + (isOpen ? "ti-chevron-up" : "ti-chevron-down")} style={{ fontSize: 14 }}></i></button>
                  </div>
                  <div className="group-card__desc">{g.desc[lang]}</div>
                  <div className="group-card__perms">{g.perms.map((p) => <span className="perm-tag" key={p}><i className="ti ti-check" style={{ fontSize: 11 }}></i>{PERM_LABEL[lang][p]}</span>)}</div>
                  <div className="group-card__foot">
                    <span><i className="ti ti-users"></i> {members.length} {lang === "pt" ? "membros" : "members"}</span>
                    <span><i className="ti ti-apps"></i> {apps.length} apps</span>
                    <button className="star-toggle" style={{ marginLeft: "auto" }} onClick={() => push((lang === "pt" ? "Editar grupo · " : "Edit group · ") + g.name[lang], "ti-pencil")}><i className="ti ti-pencil"></i></button>
                  </div>

                  {isOpen && (
                    <div className="group-detail fade-in">
                      <div className="group-detail__section">
                        <div className="group-detail__label"><i className="ti ti-users"></i>{lang === "pt" ? "Membros" : "Members"}</div>
                        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                          {members.map((u) => (
                            <div key={u.id} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                              <Avatar name={u.name} size={22} />
                              <span style={{ fontWeight: 500 }}>{u.name}</span>
                              <span className="faint mono" style={{ fontSize: 10.5 }}>{u.email}</span>
                            </div>
                          ))}
                          {members.length === 0 && <span className="faint" style={{ fontSize: 12 }}>{lang === "pt" ? "Nenhum membro" : "No members"}</span>}
                        </div>
                      </div>
                      <div className="group-detail__section">
                        <div className="group-detail__label"><i className="ti ti-apps"></i>{lang === "pt" ? "Apps restritos a este grupo" : "Apps restricted to this group"}</div>
                        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
                          {apps.map((a) => (
                            <div key={a.id} style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11.5, padding: "3px 8px", borderRadius: 8, background: "var(--surface-soft)", border: "0.5px solid var(--border)" }}>
                              <A3.Logo app={a} size={16} radius={4} />{a.name}
                            </div>
                          ))}
                          {apps.length === 0 && <span className="faint" style={{ fontSize: 12 }}>{lang === "pt" ? "Nenhum app exclusivo" : "No exclusive apps"}</span>}
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Public apps section */}
      <div style={{ marginTop: 26 }}>
        <div className="rail-head" style={{ marginBottom: 12 }}>
          <span className="rail-head__title"><i className="ti ti-world" style={{ fontSize: 17, color: "var(--teal-600)" }}></i>{lang === "pt" ? "Apps públicos" : "Public apps"}
            <span style={{ fontSize: 11, fontWeight: 400, color: "var(--text-faint)", marginLeft: 6 }}>· {lang === "pt" ? "sem grupo = visíveis a todos" : "no group = visible to everyone"}</span>
          </span>
          <span className="rail-head__count">{publicApps.length}</span>
          <span className="rail-head__line"></span>
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          {publicApps.map((a) => (
            <div key={a.id} style={{ display: "inline-flex", alignItems: "center", gap: 7, padding: "5px 10px 5px 7px", borderRadius: 9, background: "var(--surface)", border: "0.5px solid var(--border)", fontSize: 12 }}>
              <A3.Logo app={a} size={20} radius={5} />{a.name}
              <i className="ti ti-world" style={{ fontSize: 12, color: "var(--teal-600)", marginLeft: 2 }}></i>
            </div>
          ))}
          {publicApps.length === 0 && <span className="faint" style={{ fontSize: 12 }}>{lang === "pt" ? "Nenhum app público." : "No public apps."}</span>}
        </div>
      </div>
    </div>
  );
}

/* ══════════════════════════════════════ AUDIT ══════════════════════ */
const AUDIT_ACTIONS = {
  "app.created":         { cat: "app",      icon: "ti-plus",     color: "#10b981" },
  "app.updated":         { cat: "app",      icon: "ti-pencil",   color: "#3b82f6" },
  "app.deleted":         { cat: "app",      icon: "ti-trash",    color: "#ef4444" },
  "user.invited":        { cat: "user",     icon: "ti-user-plus",color: "#10b981" },
  "user.suspended":      { cat: "user",     icon: "ti-user-x",   color: "#f59e0b" },
  "credential.created":  { cat: "security", icon: "ti-key",      color: "#10b981" },
  "credential.revoked":  { cat: "security", icon: "ti-ban",      color: "#ef4444" },
  "auth.login":          { cat: "security", icon: "ti-login-2",  color: "#3b82f6" },
  "appearance.published":{ cat: "config",   icon: "ti-palette",  color: "#7c3aed" },
  "disk.pruned":         { cat: "config",   icon: "ti-eraser",   color: "#3b82f6" },
};
const SEED_AUDIT = [
  { ts: "05/06 19:42:10", actor: "Ana Ribeiro",   action: "appearance.published", target: "portal",         ip: "200.17.12.4" },
  { ts: "05/06 19:38:55", actor: "Carlos Mendes", action: "app.updated",          target: "streamlit",      ip: "200.17.12.9" },
  { ts: "05/06 18:20:03", actor: "Ana Ribeiro",   action: "user.invited",         target: "diego@ufpe.br",  ip: "200.17.12.4" },
  { ts: "05/06 17:05:41", actor: "sistema",       action: "disk.pruned",          target: "3 imgs · 2.3 GB",ip: "—" },
  { ts: "05/06 16:11:22", actor: "Beatriz Lima",  action: "app.created",          target: "sales-dashboard",ip: "200.17.12.31" },
  { ts: "05/06 15:48:09", actor: "Carlos Mendes", action: "credential.revoked",   target: "rk_live_3c0a",   ip: "200.17.12.9" },
  { ts: "05/06 14:30:17", actor: "Ana Ribeiro",   action: "auth.login",           target: "admin",          ip: "200.17.12.4" },
  { ts: "05/06 11:02:50", actor: "Felipe Araújo", action: "app.deleted",          target: "old-demo",       ip: "187.4.55.2" },
  { ts: "04/06 22:14:33", actor: "Ana Ribeiro",   action: "credential.created",   target: "CI / Deploy",    ip: "200.17.12.4" },
  { ts: "04/06 20:55:08", actor: "sistema",       action: "user.suspended",       target: "felipe@ufpe.br", ip: "—" },
];
const CAT_LABEL = {
  pt: { app: "Apps", user: "Usuários", security: "Segurança", config: "Configuração" },
  en: { app: "Apps", user: "Users",    security: "Security",   config: "Config" },
};

function AdminAudit({ t, loading, push }) {
  const lang = t._lang;
  const [q, setQ] = a3S("");
  const [cat, setCat] = a3S("all");
  const shown = SEED_AUDIT.filter((e) => {
    if (cat !== "all" && AUDIT_ACTIONS[e.action].cat !== cat) return false;
    if (q.trim() && !(e.actor + " " + e.action + " " + e.target).toLowerCase().includes(q.trim().toLowerCase())) return false;
    return true;
  });
  const Chip = ({ k, label }) => <button className={"chip" + (cat === k ? " on-all" : "")} onClick={() => setCat(cat === k ? "all" : k)}>{label}</button>;
  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 18 }}>
        <div><div className="page-title">{lang === "pt" ? "Auditoria" : "Audit log"}</div><div className="page-sub">{lang === "pt" ? "Registro de ações administrativas" : "Record of administrative actions"}</div></div>
        <button className="btn" onClick={() => push(lang === "pt" ? "Exportando CSV…" : "Exporting CSV…", "ti-download")}><i className="ti ti-download"></i>{lang === "pt" ? "Exportar" : "Export"}</button>
      </div>
      <div className="admin-sticky">
        <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
          <div className="search-pill" style={{ maxWidth: 280, flex: "0 1 280px", padding: "8px 14px" }}>
            <i className="ti ti-search"></i>
            <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={lang === "pt" ? "Buscar evento, ator, alvo…" : "Search event, actor, target…"} />
          </div>
          <Chip k="app" label={CAT_LABEL[lang].app} />
          <Chip k="user" label={CAT_LABEL[lang].user} />
          <Chip k="security" label={CAT_LABEL[lang].security} />
          <Chip k="config" label={CAT_LABEL[lang].config} />
          <div style={{ flex: 1 }}></div>
          <span className="faint tnum" style={{ fontSize: 11 }}>{shown.length} {lang === "pt" ? "eventos" : "events"}</span>
        </div>
      </div>
      {loading ? <div style={{ marginTop: 12 }}><A3.SkBlock w={"100%"} h={340} r={12} /></div> : (
        <table className="admin-table" style={{ marginTop: 12 }}>
          <thead><tr>
            <th style={{ width: 140 }}>{lang === "pt" ? "Quando" : "When"}</th>
            <th style={{ width: 170 }}>{lang === "pt" ? "Ator" : "Actor"}</th>
            <th style={{ width: 200 }}>{lang === "pt" ? "Ação" : "Action"}</th>
            <th>{lang === "pt" ? "Alvo" : "Target"}</th>
            <th style={{ width: 120 }}>IP</th>
          </tr></thead>
          <tbody>
            {shown.map((e, i) => {
              const a = AUDIT_ACTIONS[e.action];
              return (
                <tr key={i}>
                  <td className="mono faint" style={{ fontSize: 11 }}>{e.ts}</td>
                  <td>{e.actor === "sistema" || e.actor === "system"
                    ? <span style={{ display: "inline-flex", alignItems: "center", gap: 8, color: "var(--text-muted)" }}><span className="rk-avatar" style={{ width: 26, height: 26, background: "var(--surface-soft)", color: "var(--text-faint)" }}><i className="ti ti-robot" style={{ fontSize: 13 }}></i></span>{lang === "pt" ? "sistema" : "system"}</span>
                    : <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}><Avatar name={e.actor} size={26} />{e.actor}</span>}</td>
                  <td><span className="audit-action" style={{ color: a.color }}><i className={"ti " + a.icon}></i><code className="mono">{e.action}</code></span></td>
                  <td className="mono muted" style={{ fontSize: 11.5 }}>{e.target}</td>
                  <td className="mono faint" style={{ fontSize: 11 }}>{e.ip}</td>
                </tr>
              );
            })}
            {shown.length === 0 && <tr><td colSpan={5} style={{ textAlign: "center", color: "var(--text-faint)", padding: 30, fontSize: 12 }}>{lang === "pt" ? "Nenhum evento." : "No events."}</td></tr>}
          </tbody>
        </table>
      )}
    </div>
  );
}

window.RKAdmin3 = { AdminUsers, AdminGroups, AdminAudit };
