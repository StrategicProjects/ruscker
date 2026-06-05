/* global React, ReactDOM, window */
const { useState: aS, useEffect: aE, useRef: aR } = React;
const AC = window.RKC;

const VARIATIONS = {
  portal: [
    { k: "a", label: { pt: "Refinado", en: "Refined" } },
    { k: "b", label: { pt: "Compacto", en: "Compact" } },
    { k: "c", label: { pt: "Seções", en: "Sections" } },
  ],
  dashboard: [
    { k: "a", label: { pt: "Agrupado", en: "Grouped" } },
    { k: "b", label: { pt: "Cards", en: "Cards" } },
    { k: "c", label: { pt: "Operações", en: "Ops" } },
  ],
};

function load(key, def) { try { return localStorage.getItem("rk_" + key) || def; } catch (e) { return def; } }
function save(key, v) { try { localStorage.setItem("rk_" + key, v); } catch (e) {} }

function App() {
  // migrate legacy "dashboard" screen value → admin + dashboard sub-page
  const legacy = load("screen", "portal");
  const [screen, setScreen] = aS(() => (legacy === "dashboard" ? "admin" : legacy));
  const [adminPage, setAdminPage] = aS(() => load("apage", "dashboard"));
  const [editApp, setEditApp] = aS(null); // {app, isNew} | null
  const [variation, setVariation] = aS(() => load("var", "a"));
  const [theme, setTheme] = aS(() => load("theme", "light"));
  const [lang, setLang] = aS(() => load("lang", "pt"));
  const [loading, setLoading] = aS(true);
  const [prog, setProg] = aS(0);
  const [barOn, setBarOn] = aS(false);
  const [push, toastNode] = AC.useToasts();
  // Shared favorite/featured state — starred apps surface in the portal's
  // Featured carousel and in the Appearance preview.
  const [featured, setFeatured] = aS(() => new Set(window.RK.APPS.filter((a) => a.featured).map((a) => a.id)));
  const toggleFeatured = (id) => setFeatured((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });

  // Shared app-access state: Map<appId, string[]>  [] = public, else restricted to group ids
  const [appAccess, setAppAccess] = aS(() => {
    const m = new Map();
    window.RK.APPS.forEach((a) => m.set(a.id, a.accessGroups || []));
    return m;
  });
  const setAccess = (id, groups) => setAppAccess((m) => { const n = new Map(m); n.set(id, groups); return n; });

  // Simulated portal user (null = anonymous)
  const [portalUser, setPortalUser] = aS(null);

  // Portal appearance config — shared between Appearance (editor) and Portal (renderer)
  const [portalCfg, setPortalCfg] = aS({
    title: "Ruscker",
    tagline: "Catálogo de aplicações e APIs",
    accent: "#0f6e56",
    theme: "light",
    layout: "grid",
    featured: true,
    showAccess: true,
    showSearch: true,
    density: "comfortable",
    footer: "v0.1.45 · Ruscker",
    logo: "wordmark",
    logoUrl: "assets/ruscker-mark.svg",
    logoSize: 28,
    logoGap: 8,
    footerLogo: null,
    footerLogoSize: 14,
    footerLogoGap: 7,
    hero: "soft",
    cover: "tint",
    seoTitle: "Ruscker — Portal de aplicações",
    seoDesc: "Acesse aplicações e APIs interativas hospedadas com Ruscker.",
    ogImage: "assets/ruscker-mark.svg",
    analytics: "none",
    analyticsId: "",
    respectDnt: true,
    customCss: "/* CSS aplicado ao portal público */\n.rcard { border-radius: 14px; }",
    htmlTop: "",
    htmlAfterFeatured: "",
    htmlBeforeFooter: "",
    htmlFooter: "",
  });

  const timers = aR([]);

  aE(() => { document.documentElement.setAttribute("data-theme", theme); save("theme", theme); }, [theme]);
  aE(() => save("lang", lang), [lang]);
  aE(() => save("screen", screen), [screen]);
  aE(() => save("apage", adminPage), [adminPage]);
  aE(() => save("var", variation), [variation]);

  const runLoad = (p0) => {
    timers.current.forEach(clearTimeout);
    setLoading(true); setBarOn(true); setProg(p0);
    const T = [];
    T.push(setTimeout(() => setProg(58), 110));
    T.push(setTimeout(() => setProg(86), 300));
    T.push(setTimeout(() => setProg(100), 560));
    T.push(setTimeout(() => setLoading(false), 600));
    T.push(setTimeout(() => { setBarOn(false); setProg(0); }, 880));
    timers.current = T;
  };

  // Loading sequence — the core "perceived speed" demo
  aE(() => { runLoad(10); return () => timers.current.forEach(clearTimeout); }, [screen, variation, adminPage]);
  const reload = () => runLoad(12);

  // navigating the admin nav clears any open form
  const goAdmin = (p) => { setEditApp(null); setAdminPage(p); };

  const t = { ...window.RK.I18N[lang], _lang: lang };

  // variation only applies to portal & the admin dashboard
  const inForm = screen === "admin" && (adminPage === "editor" || adminPage === "import");
  const hasVars = screen === "portal" || (screen === "admin" && adminPage === "dashboard");
  const vars = screen === "portal" ? VARIATIONS.portal : VARIATIONS.dashboard;
  const curVar = vars.map((v) => v.k).includes(variation) ? variation : "a";

  const openApp = (app) => push((lang === "pt" ? "Abrindo " : "Opening ") + app.name + "…", "ti-external-link");
  const onEdit = (app, isNew) => { setEditApp({ app, isNew: !!isNew }); setAdminPage("editor"); };
  const onImport = () => { setEditApp(null); setAdminPage("import"); };
  const closeForm = () => setAdminPage("apps");

  return (
    <div className="proto-shell">
      <div className={"topbar-progress" + (barOn ? " is-active" : "")} style={{ width: prog + "%" }}></div>

      {/* ── Prototype chrome ── */}
      <div className="proto-bar">
        <div className="proto-bar__brand">
          <img src="assets/ruscker-mark.svg" alt="" />
          <span className="wm">ruscker</span>
          <span className="tag">UX proto</span>
        </div>

        <div className="proto-group">
          <span className="proto-label">{lang === "pt" ? "Tela" : "Screen"}</span>
          <div className="seg">
            <button className={screen === "portal" ? "on" : ""} onClick={() => setScreen("portal")}><i className="ti ti-layout-grid"></i>{t.portal}</button>
            <button className={screen === "admin" ? "on" : ""} onClick={() => setScreen("admin")}><i className="ti ti-server-cog"></i>Admin</button>
          </div>
        </div>

        {hasVars && (
          <div className="proto-group">
            <span className="proto-label">{lang === "pt" ? "Variação" : "Variation"}</span>
            <div className="seg seg--teal">
              {vars.map((v) => (
                <button key={v.k} className={curVar === v.k ? "on" : ""} onClick={() => setVariation(v.k)}>
                  {v.k.toUpperCase()} · {v.label[lang]}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="proto-bar__spacer"></div>

        <button className="icon-btn" title={lang === "pt" ? "Recarregar (ver skeletons)" : "Reload (replay skeletons)"} onClick={reload}><i className="ti ti-refresh"></i></button>
        <button className="icon-btn" title={lang === "pt" ? "Tema" : "Theme"} onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
          <i className={"ti " + (theme === "dark" ? "ti-moon" : "ti-sun")}></i>
        </button>
        <button className="icon-btn lang" title="Idioma / Language" onClick={() => setLang(lang === "pt" ? "en" : "pt")}>{lang.toUpperCase()}</button>
      </div>

      {/* ── Mock product ── */}
      <div className="proto-stage" style={screen === "portal" && portalCfg ? { "--teal-600": portalCfg.accent, "--teal-400": portalCfg.accent } : null}>
        <MockHeader screen={screen} adminPage={adminPage} setAdminPage={goAdmin} setScreen={setScreen}
          t={t} lang={lang} theme={theme} setTheme={setTheme} setLang={setLang} push={push} inForm={inForm} portalCfg={portalCfg} />
        <div className="mock">
          {screen === "portal" && <window.RKPortal variation={curVar} t={t} loading={loading} onOpen={openApp} featured={featured} appAccess={appAccess} portalUser={portalUser} setPortalUser={setPortalUser} portalCfg={portalCfg} />}
          {screen === "admin" && adminPage === "dashboard" && <window.RKDashboard variation={curVar} t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "apps" && <window.RKAdmin.AdminApps t={t} loading={loading} push={push} onEdit={onEdit} onImport={onImport} featured={featured} toggleFeatured={toggleFeatured} appAccess={appAccess} setAccess={setAccess} />}
          {screen === "admin" && adminPage === "media" && <window.RKAdmin.AdminMedia t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "appearance" && <window.RKAppearance t={t} loading={loading} push={push} featured={featured} portalCfg={portalCfg} setPortalCfg={setPortalCfg} />}
          {screen === "admin" && adminPage === "disk" && <window.RKAdmin2.AdminDisk t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "users" && <window.RKAdmin3.AdminUsers t={t} loading={loading} push={push} appAccess={appAccess} />}
          {screen === "admin" && adminPage === "groups" && <window.RKAdmin3.AdminGroups t={t} loading={loading} push={push} appAccess={appAccess} />}
          {screen === "admin" && adminPage === "audit" && <window.RKAdmin3.AdminAudit t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "credentials" && <window.RKAdmin2.AdminCredentials t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "logs" && <window.RKAdmin2.AdminLogs t={t} loading={loading} push={push} />}
          {screen === "admin" && adminPage === "editor" && <window.RKForms.AppEditor t={t} app={(editApp && editApp.app) || window.RK.APPS[0]} isNew={editApp ? editApp.isNew : true} onClose={closeForm} push={push} appAccess={appAccess} setAccess={setAccess} />}
          {screen === "admin" && adminPage === "import" && <window.RKForms.ImportYaml t={t} onClose={closeForm} push={push} />}
        </div>
        <Footer screen={screen} />
      </div>

      {toastNode}
    </div>
  );
}

function MockHeader({ screen, adminPage, setAdminPage, setScreen, t, lang, theme, setTheme, setLang, push, inForm, portalCfg }) {
  const ThemeLang = (
    <React.Fragment>
      <button className="chrome-icon" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}><i className={"ti " + (theme === "dark" ? "ti-moon" : "ti-sun")}></i></button>
      <button className="chrome-icon lang" style={{ fontSize: 11, fontWeight: 600 }} onClick={() => setLang(lang === "pt" ? "en" : "pt")}>{lang.toUpperCase()}</button>
    </React.Fragment>
  );

  if (screen === "portal") {
    const cfg = portalCfg || {};
    const heroBg = cfg.hero === "bold"
      ? `linear-gradient(150deg, ${cfg.accent||"var(--teal-600)"}, color-mix(in srgb, ${cfg.accent||"var(--teal-600)"} 45%, #000))`
      : cfg.hero === "soft"
      ? `linear-gradient(160deg, color-mix(in srgb, ${cfg.accent||"var(--teal-600)"} 20%, transparent), transparent 80%)`
      : "transparent";
    const heroInk = cfg.hero === "bold" ? "#fff" : "inherit";
    const showLogo = cfg.logo !== "mark";
    const logoSrc = cfg.logo === "custom" && cfg.logoUrl ? cfg.logoUrl : "assets/ruscker-mark.svg";
    return (
      <div className="lhead" style={{ background: heroBg, color: heroInk }}>
        <div className="lhead__brand" style={{ gap: cfg.logoGap || 8, alignItems: "center" }}>
          <img src={logoSrc} alt="" style={{ width: cfg.logoSize || 28, height: cfg.logoSize || 28, display: "block", flexShrink: 0 }} />
          {showLogo && (
            <div>
              <span className="wm" style={{ color: heroInk, fontSize: 17, fontWeight: 700, letterSpacing: "-0.02em" }}>{cfg.title || "Ruscker"}</span>
              {cfg.tagline && <div style={{ fontSize: 11, opacity: 0.6, lineHeight: 1.2, marginTop: 1 }}>{cfg.tagline}</div>}
            </div>
          )}
        </div>
        <div className="lhead__right">
          {ThemeLang}
          <button className="chrome-signin" style={cfg.hero === "bold" ? { borderColor: "rgba(255,255,255,.5)", color: "#fff" } : null} onClick={() => push(t.signin + "…", "ti-login-2")}><i className="ti ti-login-2"></i>{t.signin}</button>
        </div>
      </div>
    );
  }

  // admin top nav — all pages functional
  const items = [
    { id: "dashboard", icon: "ti-activity", label: lang === "pt" ? "Painel" : "Dashboard" },
    { id: "apps", icon: "ti-apps", label: "Apps" },
    { id: "appearance", icon: "ti-palette", label: lang === "pt" ? "Aparência" : "Appearance" },
    { id: "media", icon: "ti-photo", label: lang === "pt" ? "Mídia" : "Media" },
    { id: "disk", icon: "ti-database", label: lang === "pt" ? "Disco" : "Disk" },
    { id: "users", icon: "ti-users", label: lang === "pt" ? "Usuários" : "Users" },
    { id: "groups", icon: "ti-users-group", label: lang === "pt" ? "Grupos" : "Groups" },
    { id: "credentials", icon: "ti-key", label: lang === "pt" ? "Credenciais" : "Credentials" },
    { id: "audit", icon: "ti-history", label: lang === "pt" ? "Auditoria" : "Audit" },
    { id: "logs", icon: "ti-terminal-2", label: "Logs" },
  ];
  // editor/import are sub-pages of Apps — keep Apps highlighted there
  const activeId = inForm ? "apps" : adminPage;
  return (
    <div className="lhead" style={{ borderBottom: "0.5px solid var(--border)", paddingTop: 11, paddingBottom: 11 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 16, minWidth: 0 }}>
        <button className="lhead__brand" style={{ border: 0, background: "transparent", cursor: "pointer", padding: 0 }} onClick={() => setScreen("portal")} title={lang === "pt" ? "Voltar ao portal" : "Back to portal"}>
          <img src="assets/ruscker-mark.svg" alt="" style={{ width: 22, height: 22 }} /><span className="wm" style={{ fontSize: 15 }}>ruscker · admin</span>
        </button>
        <nav style={{ display: "flex", gap: 2, flexWrap: "wrap" }}>
          {items.map((it) => (
            <button key={it.id}
              className={"admin-nav" + (activeId === it.id ? " on" : "")}
              onClick={() => setAdminPage(it.id)}>
              <i className={"ti " + it.icon}></i>{it.label}
            </button>
          ))}
          <button className="admin-nav" onClick={() => setScreen("portal")}><i className="ti ti-external-link"></i>Portal</button>
        </nav>
      </div>
      <div className="lhead__right">{ThemeLang}<button className="chrome-icon"><i className="ti ti-user"></i></button></div>
    </div>
  );
}

function Footer({ screen }) {
  return (
    <div className="mock-foot mock-pad" style={{ maxWidth: 1180 }}>
      <span>v0.1.45</span>
      <span className="v"><img src="assets/ruscker-mark.svg" alt="" />{screen === "portal" ? "Ruscker" : "ruscker · admin"}</span>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
