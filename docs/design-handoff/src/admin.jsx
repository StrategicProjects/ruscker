/* global React, window */
// ── Admin pages: Apps catalog + Media library (refined style) ──
const { useState: amS, useMemo: amM } = React;
const AM = window.RKC;

const ADMIN_KIND = {
  shiny: "shiny", rmarkdown: "shiny", "shiny-for-python": "shiny",
  jupyter: "interactive", rstudio: "interactive", streamlit: "interactive",
  dash: "interactive", quarto: "interactive", voila: "interactive",
  fastapi: "api", plumber: "api",
  bokeh: "external", "ruscker-docs": "external",
};
const KIND_LABEL = {
  pt: { shiny: "SHINY", interactive: "INTERATIVO", api: "API", external: "EXTERNO" },
  en: { shiny: "SHINY", interactive: "INTERACTIVE", api: "API", external: "EXTERNAL" },
};

/* ══════════════════════════════════════ APPS ═══════════════════ */
function AdminApps({ t, loading, push, onEdit, onImport, featured, toggleFeatured, appAccess, setAccess }) {
  const APPS = window.RK.APPS;
  const [q, setQ] = amS("");
  const [kind, setKind] = amS("all");
  const [sort, setSort] = amS("recent");

  const kinds = amM(() => {
    const c = { all: APPS.length, shiny: 0, interactive: 0, api: 0, external: 0 };
    APPS.forEach((a) => { c[ADMIN_KIND[a.id]]++; });
    return c;
  }, []);

  const rows = amM(() => {
    let list = APPS.filter((a) => {
      if (kind !== "all" && ADMIN_KIND[a.id] !== kind) return false;
      if (q.trim() && !(a.id + " " + a.name).toLowerCase().includes(q.trim().toLowerCase())) return false;
      return true;
    });
    if (sort === "name") list = [...list].sort((a, b) => a.name.localeCompare(b.name));
    if (sort === "id") list = [...list].sort((a, b) => a.id.localeCompare(b.id));
    return list;
  }, [q, kind, sort]);

  const lang = t._lang;
  const GROUP_COLOR = { admin: "#ef476f", editor: "#06b6d4", viewer: "#26547c" };
  const G_LABEL = { admin: "Admin", editor: "Editor", viewer: "Viewer" };

  const KindChip = ({ k, label }) => (
    <button className={"chip" + (kind === k ? " on-all" : "")} onClick={() => setKind(k)}>{label} <span className="chip-count">{kinds[k]}</span></button>
  );

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 18 }}>
        <div>
          <div className="page-title">Apps</div>
          <div className="page-sub">{lang === "pt" ? "Catálogo de specs no banco de dados" : "Spec catalog stored in the database"}</div>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span className="faint tnum" style={{ fontSize: 12 }}>{APPS.length}</span>
          <button className="btn" onClick={onImport}><i className="ti ti-file-import"></i>{lang === "pt" ? "Importar YAML" : "Import YAML"}</button>
          <button className="btn btn--primary" onClick={() => onEdit(null, true)}><i className="ti ti-plus"></i>{lang === "pt" ? "Novo app" : "Add app"}</button>
        </div>
      </div>

      <div className="admin-sticky">
        <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
          <div className="search-pill" style={{ maxWidth: 280, flex: "0 1 280px", padding: "8px 14px" }}>
            <i className="ti ti-search"></i>
            <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={lang === "pt" ? "Buscar app…" : "Search app…"} />
          </div>
          <KindChip k="all" label={t.all} />
          <KindChip k="shiny" label="Shiny" />
          <KindChip k="interactive" label={lang === "pt" ? "Interativo" : "Interactive"} />
          <KindChip k="api" label="API" />
          <KindChip k="external" label={lang === "pt" ? "Externo" : "External"} />
          <div style={{ flex: 1 }}></div>
          <button className="sort-btn" onClick={() => setSort(sort === "recent" ? "name" : sort === "name" ? "id" : "recent")}>
            <i className="ti ti-arrows-sort"></i>{sort === "recent" ? t.recent : sort === "name" ? t.name : "ID"}
          </button>
        </div>
      </div>

      <table className="admin-table" style={{ marginTop: 12 }}>
        <thead>
          <tr>
            <th style={{ width: 130 }}>ID</th>
            <th>{lang === "pt" ? "Nome" : "Name"}</th>
            <th style={{ width: 130 }}>{lang === "pt" ? "Tipo" : "Kind"}</th>
            <th style={{ width: 180 }}>{lang === "pt" ? "Acesso" : "Access"}</th>
            <th style={{ width: 90 }}>{lang === "pt" ? "Estado" : "State"}</th>
            <th style={{ width: 120 }}>{lang === "pt" ? "Atualizado" : "Updated"}</th>
            <th style={{ width: 80 }}>{lang === "pt" ? "Versão" : "Version"}</th>
            <th style={{ width: 130, textAlign: "right" }}>{lang === "pt" ? "Ações" : "Actions"}</th>
          </tr>
        </thead>
        <tbody>
          {loading
            ? Array.from({ length: 8 }).map((_, i) => (
                <tr key={i}><td colSpan={7} style={{ padding: 0 }}><div style={{ padding: "11px 12px" }}><AM.SkBlock w={"100%"} h={14} /></div></td></tr>
              ))
            : rows.map((a) => {
                const ak = ADMIN_KIND[a.id];
                const groups = appAccess ? (appAccess.get(a.id) ?? a.accessGroups ?? []) : (a.accessGroups || []);
                const isPublic = groups.length === 0;
                const toggleG = (g) => {
                  if (!setAccess) return;
                  const next = groups.includes(g) ? groups.filter((x) => x !== g) : [...groups, g];
                  setAccess(a.id, next);
                };
                return (
                  <React.Fragment key={a.id}>
                    <tr style={{ cursor: "pointer" }} onClick={() => onEdit(a)}>
                      <td className="mono muted">{a.id}</td>
                      <td><span style={{ display: "inline-flex", alignItems: "center", gap: 10 }}><AM.Logo app={a} size={26} radius={6} /><span style={{ fontWeight: 500 }}>{a.name}</span></span></td>
                      <td><span className={"pill pill-kind-" + ak}>{KIND_LABEL[lang][ak]}</span></td>
                      <td style={{ cursor: "default" }} onClick={(e) => e.stopPropagation()}>
                        <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                          {isPublic
                            ? <span style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11, color: "var(--teal-600)" }}><i className="ti ti-world" style={{ fontSize: 13 }}></i>{lang === "pt" ? "público" : "public"}</span>
                            : groups.map((g) => <span key={g} className="group-badge" style={{ background: `color-mix(in srgb, ${GROUP_COLOR[g]} 16%, transparent)`, color: GROUP_COLOR[g] }}>{G_LABEL[g]}</span>)}
                        </div>
                      </td>
                      <td><span className="pill pill-state-active">{lang === "pt" ? "ATIVO" : "ACTIVE"}</span></td>
                      <td className="muted tnum" style={{ fontSize: 11.5 }}>{a.updated}/2026</td>
                      <td onClick={(e) => e.stopPropagation()}>
                        <div style={{ display: "flex", gap: 2, justifyContent: "flex-end", alignItems: "center" }}>
                          <button className={"star-toggle" + (featured.has(a.id) ? " is-on" : "")} title={featured.has(a.id) ? (lang === "pt" ? "Remover destaque" : "Unfeature") : (lang === "pt" ? "Destacar" : "Feature")}
                            onClick={() => { toggleFeatured(a.id); push((featured.has(a.id) ? (lang === "pt" ? "Removido dos destaques" : "Unfeatured") : (lang === "pt" ? "Adicionado aos destaques" : "Featured")) + " · " + a.name, featured.has(a.id) ? "ti-star-off" : "ti-star-filled"); }}>
                            <AM.StarIcon on={featured.has(a.id)} size={17} />
                          </button>
                          <button className="star-toggle" title={lang === "pt" ? "Editar" : "Edit"} onClick={() => onEdit(a)}><i className="ti ti-pencil"></i></button>
                          <button className="star-toggle" title={lang === "pt" ? "Duplicar" : "Duplicate"} onClick={() => push((lang === "pt" ? "Duplicado · " : "Duplicated · ") + a.name, "ti-copy")}><i className="ti ti-copy"></i></button>
                        </div>
                      </td>
                    </tr>

                  </React.Fragment>
                );
              })}
        </tbody>
      </table>
      {!loading && rows.length === 0 && (
        <div style={{ textAlign: "center", padding: "48px", color: "var(--text-muted)" }}>
          <i className="ti ti-search-off" style={{ fontSize: 28, color: "var(--text-faint)" }}></i>
          <div style={{ marginTop: 10, fontSize: 13 }}>{t.noResults}</div>
        </div>
      )}
    </div>
  );
}

/* ══════════════════════════════════════ MEDIA ═════════════════ */
function AdminMedia({ t, loading, push }) {
  const APPS = window.RK.APPS;
  const lang = t._lang;
  const [q, setQ] = amS("");
  const [drag, setDrag] = amS(false);

  const items = amM(() => {
    const sizes = ["396 B", "5.0 KB", "10.6 KB", "6.1 KB", "3.6 KB", "5.1 KB", "820 B", "7.2 KB", "4.5 KB", "9.2 KB", "3.2 KB", "8.4 KB", "5.2 KB"];
    return APPS.map((a, i) => ({ id: a.id, name: a.id + ".svg", src: a.logo, accent: a.accent, mono: a.mono, size: sizes[i % sizes.length] }));
  }, []);
  const filtered = items.filter((m) => !q.trim() || m.name.toLowerCase().includes(q.trim().toLowerCase()));

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 18 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Biblioteca de mídia" : "Media library"}</div>
          <div className="page-sub">{lang === "pt" ? "PNG, JPEG e WebP são convertidos para WebP. SVG passa direto." : "PNG, JPEG and WebP are converted to WebP. SVG passes through."}</div>
        </div>
        <span className="faint tnum" style={{ fontSize: 13 }}>{items.length}</span>
      </div>

      <div className={"image-upload-zone" + (drag ? " is-dragover" : "")}
        onDragOver={(e) => { e.preventDefault(); setDrag(true); }}
        onDragLeave={() => setDrag(false)}
        onDrop={(e) => { e.preventDefault(); setDrag(false); push(lang === "pt" ? "Imagem enviada" : "Image uploaded", "ti-photo-up"); }}>
        <button className="btn btn--primary" onClick={() => push(lang === "pt" ? "Selecionar arquivo…" : "Choose file…", "ti-photo")}><i className="ti ti-photo-plus"></i>{lang === "pt" ? "Escolher imagem" : "Choose image"}</button>
        <span className="image-upload-hint">{lang === "pt" ? "Clique ou arraste · PNG · JPEG · WebP · SVG · até 10 MB" : "Click or drop · PNG · JPEG · WebP · SVG · up to 10 MB"}</span>
      </div>

      <div className="search-pill" style={{ maxWidth: 320, margin: "18px 0 0", padding: "8px 14px" }}>
        <i className="ti ti-search"></i>
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={lang === "pt" ? "Buscar imagens…" : "Search images…"} />
      </div>

      {loading ? (
        <div className="media-grid" style={{ marginTop: 16 }}>{Array.from({ length: 8 }).map((_, i) => (
          <div className="image-tile" key={i}><div className="sk" style={{ height: 120, borderRadius: 0 }}></div><div style={{ padding: "8px 10px" }}><AM.SkBlock w={"70%"} h={9} /></div></div>
        ))}</div>
      ) : (
        <div className="media-grid stagger" style={{ marginTop: 16 }}>
          {filtered.map((m, i) => (
            <div className="image-tile" key={m.id} style={{ animationDelay: Math.min(i, 10) * 12 + "ms" }}>
              <button className="image-tile-delete" title={lang === "pt" ? "Remover" : "Delete"} onClick={() => push((lang === "pt" ? "Removida · " : "Deleted · ") + m.name, "ti-trash")}><i className="ti ti-trash"></i></button>
              <div className="image-tile-frame" style={{ background: `color-mix(in srgb, ${m.accent} 9%, var(--surface-soft))` }}>
                {m.src ? <img src={m.src} alt={m.name} onError={(e) => { e.target.style.display = "none"; }} /> : <div style={{ width: "100%", height: "100%", display: "flex", alignItems: "center", justifyContent: "center", color: m.accent, fontWeight: 600, fontSize: 22 }}>{m.mono}</div>}
              </div>
              <div className="image-tile-meta">
                <div className="image-tile-name">{m.name}</div>
                <div className="image-tile-sub"><span>{m.size}</span><span>·</span><span>image/svg+xml</span></div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

window.RKAdmin = { AdminApps, AdminMedia };
