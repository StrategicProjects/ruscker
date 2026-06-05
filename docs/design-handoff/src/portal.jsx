/* global React, window */
// ── Portal (public landing) — 3 variations ──
const { useState: uS, useMemo: uM, useRef: uR, useEffect: uE } = React;
const C = window.RKC;

/* ── UserBanner: simulated "logged in as" bar at the top of the portal ──
   Shows access context: anonymous vs. specific user (+ their groups).
   This is a prototype control — in production, this is real auth. */
function UserBanner({ t, portalUser, setPortalUser }) {
  const lang = t._lang;
  const [open, setOpen] = uS(false);
  const users = (window.RUSCKER_USERS || []).filter((u) => u.status !== "suspended");
  const GROUP_COLOR = { admin: "#ef476f", editor: "#06b6d4", viewer: "#26547c" };
  return (
    <div style={{ display: "flex", justifyContent: "flex-end", padding: "6px 0 2px", position: "relative" }}>
      <button
        onClick={() => setOpen((o) => !o)}
        style={{
          display: "inline-flex", alignItems: "center", gap: 6,
          padding: "4px 10px 4px 8px", borderRadius: 999,
          border: "0.5px solid var(--border)",
          background: portalUser ? "color-mix(in srgb, var(--teal-600) 8%, var(--surface))" : "var(--surface)",
          color: "var(--text-muted)", fontSize: 11.5, cursor: "pointer", fontFamily: "inherit",
          transition: "all .12s",
        }}
      >
        <i className={"ti " + (portalUser ? "ti-user-circle" : "ti-user-off")} style={{ fontSize: 14, color: portalUser ? "var(--teal-600)" : "var(--text-faint)" }}></i>
        <span style={{ fontWeight: portalUser ? 500 : 400, color: portalUser ? "var(--text)" : "var(--text-faint)" }}>
          {portalUser ? portalUser.name : (lang === "pt" ? "Anônimo" : "Anonymous")}
        </span>
        {portalUser && (portalUser.groups || []).map((g) => (
          <span key={g} style={{ width: 6, height: 6, borderRadius: "50%", background: GROUP_COLOR[g], flexShrink: 0 }}></span>
        ))}
        <i className="ti ti-chevron-down" style={{ fontSize: 11, opacity: 0.5 }}></i>
      </button>

      {open && (
        <div className="fade-in" style={{
          position: "absolute", top: "calc(100% + 4px)", right: 0, zIndex: 50,
          background: "var(--surface)", border: "0.5px solid var(--border)",
          borderRadius: 10, boxShadow: "var(--shadow-lg)", minWidth: 200, overflow: "hidden",
        }}>
          <div style={{ padding: "8px 12px 6px", fontSize: 10, textTransform: "uppercase", letterSpacing: ".06em", color: "var(--text-faint)" }}>
            {lang === "pt" ? "Simular usuário" : "Simulate user"}
          </div>
          {[null, ...users].map((u) => (
            <button key={u ? u.id : "anon"}
              onClick={() => { setPortalUser(u); setOpen(false); }}
              style={{
                display: "flex", alignItems: "center", gap: 8, width: "100%",
                padding: "7px 12px", border: 0, background: (portalUser?.id === u?.id || (!portalUser && !u)) ? "var(--surface-soft)" : "transparent",
                cursor: "pointer", fontSize: 12.5, fontFamily: "inherit", color: "var(--text)",
                textAlign: "left",
              }}
            >
              <i className={"ti " + (u ? "ti-user-circle" : "ti-world")} style={{ fontSize: 15, color: u ? "var(--teal-600)" : "var(--text-faint)", flexShrink: 0 }}></i>
              <span style={{ flex: 1, fontWeight: (!portalUser && !u) || portalUser?.id === u?.id ? 600 : 400 }}>
                {u ? u.name : (lang === "pt" ? "Anônimo (não logado)" : "Anonymous")}
              </span>
              {u && (u.groups || []).map((g) => (
                <span key={g} style={{ width: 6, height: 6, borderRadius: "50%", background: GROUP_COLOR[g], flexShrink: 0 }}></span>
              ))}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function Portal({ variation, t, loading, onOpen, featured, appAccess, portalUser, setPortalUser, portalCfg }) {
  const APPS = window.RK.APPS;
  const [q, setQ] = uS("");
  const [kind, setKind] = uS("all");
  const [access, setAccess] = uS("all"); // all | free | restricted
  const [sort, setSort] = uS("recent");

  // ⌘K / Ctrl-K / "/" focuses search — keyboard-fast catalog navigation
  uE(() => {
    const onKey = (e) => {
      const tag = (e.target.tagName || "").toLowerCase();
      if (((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") || (e.key === "/" && tag !== "input" && tag !== "textarea")) {
        e.preventDefault();
        const el = document.getElementById("rk-portal-search");
        if (el) el.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const counts = uM(() => {
    const visible = APPS.filter((a) => {
      const groups = appAccess ? (appAccess.get(a.id) ?? a.accessGroups ?? []) : (a.accessGroups || []);
      if (groups.length === 0) return true;
      if (!portalUser) return false;
      return groups.some((g) => (portalUser.groups || []).includes(g));
    });
    const c = { all: visible.length, app: 0, api: 0, package: 0, report: 0, free: 0, restricted: 0 };
    visible.forEach((a) => { c[a.kind]++; a.locked ? c.restricted++ : c.free++; });
    return c;
  }, [userKey, accessKey]);

  // Convert appAccess Map to a stable key for memo deps
  const accessKey = appAccess ? JSON.stringify([...appAccess.entries()].map(([k,v])=>k+':'+v.join(',')).sort()) : '';
  const userKey = portalUser ? portalUser.id : 'anon';
  const featKey = featured ? [...featured].sort().join(',') : '';

  const filtered = uM(() => {
    let list = APPS.filter((a) => {
      // Access-group check: [] = public; non-empty = restricted to those groups
      const groups = appAccess ? (appAccess.get(a.id) ?? a.accessGroups ?? []) : (a.accessGroups || []);
      const isPublic = groups.length === 0;
      if (!isPublic) {
        if (!portalUser) return false; // anonymous: can't see restricted
        const userGroups = portalUser.groups || [];
        if (!groups.some((g) => userGroups.includes(g))) return false;
      }
      if (kind !== "all" && a.kind !== kind) return false;
      if (access === "free" && a.locked) return false;
      if (access === "restricted" && !a.locked) return false;
      if (q.trim()) {
        const s = (a.name + " " + a.desc[t._lang] + " " + a.subject).toLowerCase();
        if (!s.includes(q.trim().toLowerCase())) return false;
      }
      return true;
    });
    if (sort === "name") list = [...list].sort((a, b) => a.name.localeCompare(b.name));
    return list;
  }, [q, kind, access, sort, t._lang, userKey, accessKey, featKey]);

  const hasFilters = q.trim() || kind !== "all" || access !== "all";
  const clear = () => { setQ(""); setKind("all"); setAccess("all"); };

  const filterProps = { t, q, setQ, kind, setKind, access, setAccess, counts, sort, setSort, featured, appAccess, portalUser, setPortalUser, portalCfg };

  if (variation === "b") return <PortalCompact {...filterProps} filtered={filtered} loading={loading} onOpen={onOpen} hasFilters={hasFilters} clear={clear} />;
  if (variation === "c") return <PortalSections {...filterProps} loading={loading} onOpen={onOpen} hasFilters={hasFilters} clear={clear} filtered={filtered} />;
  return <PortalRefined {...filterProps} filtered={filtered} loading={loading} onOpen={onOpen} hasFilters={hasFilters} clear={clear} />;
}

/* ── shared filter bar (chips) ─────────────────────────────────── */
function FilterChips({ t, kind, setKind, access, setAccess, counts }) {
  const Chip = ({ k, label }) => (
    <button className={"chip" + (kind === k ? (k === "all" ? " on-all" : " on-" + k) : "")} onClick={() => setKind(k)}>
      {label} <span className="chip-count">{counts[k] ?? 0}</span>
    </button>
  );
  return (
    <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 6 }}>
      <Chip k="all" label={t.all} />
      <Chip k="app" label={t.apps} />
      <Chip k="api" label={t.apis} />
      <Chip k="package" label={t.packages} />
      <Chip k="report" label={t.reports} />
      <span className="chip-divider"></span>
      <button className={"access-chip" + (access === "free" ? " on" : "")} onClick={() => setAccess(access === "free" ? "all" : "free")}>
        <i className="ti ti-lock-open" style={{ fontSize: 12, color: access === "free" ? "inherit" : "var(--lock-open)" }}></i>{t.free}
      </button>
      <button className={"access-chip" + (access === "restricted" ? " on" : "")} onClick={() => setAccess(access === "restricted" ? "all" : "restricted")}>
        <i className="ti ti-lock" style={{ fontSize: 12, color: access === "restricted" ? "inherit" : "var(--lock)" }}></i>{t.restricted}
      </button>
    </div>
  );
}

function SearchBar({ t, q, setQ, sort, setSort, long, inputId }) {
  return (
    <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
      <div className="search-pill">
        <i className="ti ti-search"></i>
        <input id={inputId} value={q} onChange={(e) => setQ(e.target.value)} placeholder={long ? t.searchLong : t.search} />
        <span className="kbd">⌘ K</span>
      </div>
      <button className="sort-btn" onClick={() => setSort(sort === "recent" ? "name" : "recent")}>
        <i className="ti ti-arrows-sort"></i>{sort === "recent" ? t.recent : t.name}
        <i className="ti ti-chevron-down"></i>
      </button>
    </div>
  );
}

function ResultsLine({ t, n, hasFilters, clear }) {
  return (
    <div style={{ fontSize: 11.5, color: "var(--text-faint)", display: "flex", alignItems: "center", gap: 8 }}>
      <span className="tnum">{n} {t.results}</span>
      {hasFilters && (
        <button onClick={clear} style={{ fontSize: 11, padding: "2px 8px", color: "var(--teal-600)", background: "transparent", border: 0, cursor: "pointer", display: "inline-flex", alignItems: "center", gap: 4 }}>
          <i className="ti ti-x" style={{ fontSize: 12 }}></i>{t.clear}
        </button>
      )}
    </div>
  );
}

function NoResults({ t }) {
  return (
    <div style={{ textAlign: "center", padding: "60px 20px", color: "var(--text-muted)" }}>
      <i className="ti ti-mood-empty" style={{ fontSize: 34, color: "var(--text-faint)" }}></i>
      <div style={{ fontSize: 15, fontWeight: 500, marginTop: 12, color: "var(--text)" }}>{t.noResults}</div>
      <div style={{ fontSize: 12.5, marginTop: 4 }}>{t.noResultsSub}</div>
    </div>
  );
}

/* ── HtmlSlot: renders a named HTML injection point ── */
function HtmlSlot({ html }) {
  if (!html || !html.trim()) return null;
  return <div className="html-slot fade-in" dangerouslySetInnerHTML={{ __html: html }} />;
}

/* ══════════════════════════════════════════════════════════════
   VARIATION A — Refined (faithful + skeletons + instant filter)
   ══════════════════════════════════════════════════════════════ */
function FeaturedCarousel({ t, loading, onOpen, featured, appAccess, portalUser, portalCfg }) {
  const userKey2 = portalUser ? portalUser.id : 'anon';
  const feat = uM(() => window.RK.APPS.filter((a) => {
    if (!featured || !featured.has(a.id)) return false;
    const groups = appAccess ? (appAccess.get(a.id) ?? a.accessGroups ?? []) : (a.accessGroups || []);
    if (groups.length === 0) return true;
    if (!portalUser) return false;
    return groups.some((g) => (portalUser.groups || []).includes(g));
  }), [userKey2, featured, appAccess]);
  const [page, setPage] = uS(0);
  const per = 3, pages = Math.ceil((feat || []).length / per);
  if (!loading && (!feat || feat.length === 0)) return null;
  const card = 256 + 14;
  return (
    <div className="feat">
      <div className="feat-head"><i className="ti ti-star"></i>{t.featured}</div>
      <div className="feat-stage">
        <button className="feat-arrow" disabled={page === 0} onClick={() => setPage(Math.max(0, page - 1))}><i className="ti ti-chevron-left"></i></button>
        <div className="feat-viewport">
          <div className="feat-track" style={{ transform: `translateX(-${page * per * card}px)` }}>
            {loading
              ? Array.from({ length: 3 }).map((_, i) => <div key={i} style={{ flex: "0 0 auto" }}><C.SkCard /></div>)
              : feat.map((a) => <div key={a.id} style={{ flex: "0 0 auto" }}><C.AppCard app={a} t={t} onOpen={onOpen} coverStyle={portalCfg && portalCfg.cover} /></div>)}
          </div>
        </div>
        <button className="feat-arrow" disabled={page >= pages - 1} onClick={() => setPage(Math.min(pages - 1, page + 1))}><i className="ti ti-chevron-right"></i></button>
      </div>
    </div>
  );
}

function PortalRefined({ t, q, setQ, kind, setKind, access, setAccess, counts, sort, setSort, filtered, loading, onOpen, hasFilters, clear, featured, appAccess, portalUser, setPortalUser, portalCfg }) {
  return (
    <div className="mock-pad" style={{ paddingTop: 8, paddingBottom: 40, "--rk-accent": portalCfg ? portalCfg.accent : undefined }}>
      {portalCfg && portalCfg.htmlTop && <HtmlSlot html={portalCfg.htmlTop} />}
      <UserBanner t={t} portalUser={portalUser} setPortalUser={setPortalUser} />
      {(portalCfg ? portalCfg.featured : true) && <FeaturedCarousel t={t} loading={loading} onOpen={onOpen} featured={featured} appAccess={appAccess} portalUser={portalUser} portalCfg={portalCfg} />}
      <div className="portal-sticky">
        {(!portalCfg || portalCfg.showSearch) && <SearchBar t={t} q={q} setQ={setQ} sort={sort} setSort={setSort} long inputId="rk-portal-search" />}
        {(!portalCfg || portalCfg.showAccess) && <FilterChips t={t} kind={kind} setKind={setKind} access={access} setAccess={setAccess} counts={counts} />}
        <ResultsLine t={t} n={loading ? "…" : filtered.length} hasFilters={hasFilters} clear={clear} />
      </div>
      {loading ? (
        <div className="cards-grid">{Array.from({ length: 8 }).map((_, i) => <C.SkCard key={i} />)}</div>
      ) : filtered.length === 0 ? <NoResults t={t} /> : (
        <div className="cards-grid stagger" key={kind + access + sort + q}>
          {filtered.map((a, i) => <div key={a.id} style={{ animationDelay: Math.min(i, 10) * 10 + "ms" }}><C.AppCard app={a} t={t} onOpen={onOpen} /></div>)}
        </div>
      )}
      {portalCfg && portalCfg.htmlBeforeFooter && <HtmlSlot html={portalCfg.htmlBeforeFooter} />}
    </div>
  );
}

/* ══════════════════════════════════════════════════════════════
   VARIATION B — Compact: left nav rail + dense rows
   ══════════════════════════════════════════════════════════════ */
function HeroFeatured({ t, loading, onOpen, featured }) {
  const feat = window.RK.APPS.filter((a) => featured && featured.has(a.id));
  const [i, setI] = uS(0);
  const a = feat[i] || feat[0];
  if (!loading && feat.length === 0) return null;
  if (loading) {
    return (
      <div className="hero-feat">
        <C.SkBlock w={200} h={160} r={10} />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", gap: 12 }}>
          <C.SkBlock w={120} h={10} /><C.SkBlock w={"50%"} h={22} /><C.SkBlock w={"80%"} h={10} /><C.SkBlock w={140} h={34} r={999} />
        </div>
      </div>
    );
  }
  return (
    <div className="hero-feat fade-in">
      <div className="hero-feat__art" style={{ width: 200, background: `color-mix(in srgb, ${a.accent} 14%, var(--surface))`, color: a.accent }}>{a.mono}</div>
      <div className="hero-feat__body">
        <div className="hero-feat__kicker"><i className="ti ti-star"></i>{t.featured}</div>
        <div className="hero-feat__title">{a.name}</div>
        <div className="hero-feat__desc">{a.desc[t._lang]}</div>
        <button className="hero-feat__cta" onClick={() => onOpen(a)}>{t.openDocs}<i className="ti ti-arrow-right"></i></button>
        <div className="hero-feat__dots">{feat.map((_, k) => <span key={k} className={k === i ? "on" : ""} onClick={() => setI(k)}></span>)}</div>
      </div>
    </div>
  );
}

function PortalCompact({ t, q, setQ, kind, setKind, access, setAccess, counts, sort, setSort, filtered, loading, onOpen, hasFilters, clear, featured, appAccess, portalUser, portalCfg }) {
  const NavItem = ({ k, icon, label, count }) => (
    <button className={"navrail__item" + (kind === k ? " on" : "")} onClick={() => setKind(k)}>
      <i className={"ti " + icon}></i>{label}<span className="count">{count}</span>
    </button>
  );
  return (
    <div className="mock-pad" style={{ paddingTop: 8, paddingBottom: 40 }}>
      <div className="split">
        <aside className="navrail">
          <div className="navrail__group">{t.categories}</div>
          <NavItem k="all" icon="ti-stack-2" label={t.all} count={counts.all} />
          <NavItem k="app" icon="ti-browser" label={t.apps} count={counts.app} />
          <NavItem k="api" icon="ti-api" label={t.apis} count={counts.api} />
          <NavItem k="package" icon="ti-package" label={t.packages} count={counts.package} />
          <NavItem k="report" icon="ti-file-text" label={t.reports} count={counts.report} />
          <div className="navrail__group">{t.access}</div>
          <button className={"navrail__item" + (access === "free" ? " on" : "")} onClick={() => setAccess(access === "free" ? "all" : "free")}>
            <i className="ti ti-lock-open"></i>{t.free}<span className="count">{counts.free}</span>
          </button>
          <button className={"navrail__item" + (access === "restricted" ? " on" : "")} onClick={() => setAccess(access === "restricted" ? "all" : "restricted")}>
            <i className="ti ti-lock"></i>{t.restricted}<span className="count">{counts.restricted}</span>
          </button>
        </aside>
        <main style={{ minWidth: 0 }}>
          <FeaturedCarousel t={t} loading={loading} onOpen={onOpen} featured={featured} appAccess={appAccess} portalUser={portalUser} portalCfg={portalCfg} />
          <div style={{ display: "flex", flexDirection: "column", gap: 12, marginBottom: 14 }}>
            <SearchBar t={t} q={q} setQ={setQ} sort={sort} setSort={setSort} long />
            <ResultsLine t={t} n={loading ? "…" : filtered.length} hasFilters={hasFilters} clear={clear} />
          </div>
          {loading ? (
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>{Array.from({ length: 7 }).map((_, i) => <C.SkRow key={i} />)}</div>
          ) : filtered.length === 0 ? <NoResults t={t} /> : (
            <div className="stagger" key={kind + access + sort + q} style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {filtered.map((a, i) => <div key={a.id} style={{ animationDelay: Math.min(i, 10) * 9 + "ms" }}><C.AppRow app={a} t={t} onOpen={onOpen} /></div>)}
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

/* ══════════════════════════════════════════════════════════════
   VARIATION C — Sectioned rails (app-store feel)
   ══════════════════════════════════════════════════════════════ */
function PortalSections({ t, q, setQ, kind, setKind, access, setAccess, counts, sort, setSort, filtered, loading, onOpen, hasFilters, clear, featured, appAccess, portalUser, portalCfg }) {
  const groupsDef = [
    { k: "app", icon: "ti-browser", label: t.apps },
    { k: "api", icon: "ti-api", label: t.apis },
    { k: "package", icon: "ti-package", label: t.packages },
    { k: "report", icon: "ti-file-text", label: t.reports },
  ];
  const searching = q.trim() || access !== "all";
  return (
    <div className="mock-pad" style={{ paddingTop: 8, paddingBottom: 40 }}>
      <FeaturedCarousel t={t} loading={loading} onOpen={onOpen} featured={featured} appAccess={appAccess} portalUser={portalUser} portalCfg={portalCfg} />
      <div style={{ display: "flex", flexDirection: "column", gap: 12, marginBottom: 22 }}>
        <SearchBar t={t} q={q} setQ={setQ} sort={sort} setSort={setSort} long />
        <FilterChips t={t} kind={kind} setKind={setKind} access={access} setAccess={setAccess} counts={counts} />
      </div>

      {loading ? (
        [0, 1].map((s) => (
          <div className="rail-section" key={s}>
            <div className="rail-head"><C.SkBlock w={140} h={16} /></div>
            <div className="rail">{Array.from({ length: 4 }).map((_, i) => <C.SkCard key={i} />)}</div>
          </div>
        ))
      ) : searching || kind !== "all" ? (
        <div>
          <div className="rail-head" style={{ marginBottom: 14 }}>
            <span className="rail-head__title"><i className="ti ti-search" style={{ fontSize: 16 }}></i>{filtered.length} {t.results}</span>
            <span className="rail-head__line"></span>
            {hasFilters && <button onClick={clear} style={{ fontSize: 11, color: "var(--teal-600)", background: "transparent", border: 0, cursor: "pointer" }}>{t.clear}</button>}
          </div>
          {filtered.length === 0 ? <NoResults t={t} /> :
            <div className="cards-grid stagger" key={kind + access + q}>{filtered.map((a, i) => <div key={a.id} style={{ animationDelay: Math.min(i, 10) * 10 + "ms" }}><C.AppCard app={a} t={t} onOpen={onOpen} /></div>)}</div>}
        </div>
      ) : (
        groupsDef.map((g) => {
          const items = filtered.filter((a) => a.kind === g.k);
          if (!items.length) return null;
          return (
            <div className="rail-section fade-in" key={g.k}>
              <div className="rail-head">
                <span className="rail-head__title"><i className={"ti " + g.icon} style={{ fontSize: 17, color: window.RK.KIND[g.k].color }}></i>{g.label}</span>
                <span className="rail-head__count">{items.length}</span>
                <span className="rail-head__line"></span>
                <button onClick={() => setKind(g.k)} style={{ fontSize: 11.5, color: "var(--text-muted)", background: "transparent", border: 0, cursor: "pointer", display: "inline-flex", gap: 4, alignItems: "center" }}>
                  {t.all}<i className="ti ti-chevron-right" style={{ fontSize: 13 }}></i>
                </button>
              </div>
              <div className="rail">{items.map((a) => <C.AppCard key={a.id} app={a} t={t} onOpen={onOpen} />)}</div>
            </div>
          );
        })
      )}
    </div>
  );
}

window.RKPortal = Portal;
