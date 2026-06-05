/* global React, window */
// ── Admin: Landing/Portal appearance configurator (live preview) ──
const { useState: apS, useRef: apR } = React;
const AP = window.RKC;

const BRAND_COLORS = ["#0f6e56", "#447099", "#26547c", "#059487", "#ef476f", "#7c3aed"];

// Media assets available to pick from (mirrors the Media library):
// the Ruscker mark + every unique app logo.
function mediaAssets() {
  const seen = new Set();
  const list = [{ id: "ruscker-mark", name: "ruscker-mark.svg", src: "assets/ruscker-mark.svg", accent: "#0f6e56", mono: "Ru" }];
  seen.add("assets/ruscker-mark.svg");
  window.RK.APPS.forEach((a) => {
    if (a.logo && !seen.has(a.logo)) { seen.add(a.logo); list.push({ id: a.id, name: a.id + ".svg", src: a.logo, accent: a.accent, mono: a.mono }); }
  });
  return list;
}

function MediaPicker({ value, onPick, lang }) {
  const items = mediaAssets();
  return (
    <div className="media-picker">
      {items.map((m) => (
        <button key={m.id} className={"media-pick" + (value === m.src ? " on" : "")} title={m.name} onClick={() => onPick(m.src)}
          style={{ background: `color-mix(in srgb, ${m.accent} 9%, var(--surface-soft))` }}>
          <img src={m.src} alt={m.name} onError={(e) => { e.target.style.display = "none"; }} />
          {value === m.src && <span className="media-pick__check"><i className="ti ti-check"></i></span>}
        </button>
      ))}
    </div>
  );
}

function Appearance({ t, loading, push, featured, portalCfg, setPortalCfg }) {
  const lang = t._lang;
  const cfg = portalCfg;
  const set = (k, v) => setPortalCfg((c) => ({ ...c, [k]: v }));

  const layouts = [
    { k: "grid", icon: "ti-layout-grid", label: lang === "pt" ? "Grade" : "Grid" },
    { k: "list", icon: "ti-list", label: lang === "pt" ? "Lista" : "List" },
    { k: "sections", icon: "ti-layout-rows", label: lang === "pt" ? "Seções" : "Sections" },
  ];

  if (loading) {
    return <div className="mock-pad" style={{ paddingTop: 16 }}><AP.SkBlock w={"100%"} h={420} r={12} /></div>;
  }

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 22 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Aparência do portal" : "Portal appearance"}</div>
          <div className="page-sub">{lang === "pt" ? "Configure como o portal público é exibido aos visitantes" : "Configure how the public portal looks to visitors"}</div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn btn--primary" onClick={() => push(lang === "pt" ? "Aparência publicada" : "Appearance published", "ti-check")}><i className="ti ti-device-floppy"></i>{lang === "pt" ? "Publicar" : "Publish"}</button>
        </div>
      </div>

      <div className="appear-grid">
        {/* ── controls ── */}
        <div className="editor-form">
          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Identidade" : "Identity"}</div>
            <label className="form-field"><span className="form-label">{lang === "pt" ? "Título do portal" : "Portal title"}</span>
              <input className="form-input" value={cfg.title} onChange={(e) => set("title", e.target.value)} /></label>
            <label className="form-field"><span className="form-label">{lang === "pt" ? "Subtítulo" : "Tagline"}</span>
              <input className="form-input" value={cfg.tagline} onChange={(e) => set("tagline", e.target.value)} /></label>
            <label className="form-field"><span className="form-label">{lang === "pt" ? "Rodapé" : "Footer"}</span>
              <input className="form-input mono" value={cfg.footer} onChange={(e) => set("footer", e.target.value)} /></label>
            <div className="toggle-row" style={{ borderBottom: 0, paddingBottom: 0 }} onClick={() => set("footerLogo", cfg.footerLogo ? null : "assets/ruscker-mark.svg")}>
              <div><div className="toggle-row__label">{lang === "pt" ? "Logo no rodapé" : "Footer logo"}</div><div className="toggle-row__hint">{lang === "pt" ? "Exibe um logo da mídia ao lado do texto" : "Shows a media logo beside the text"}</div></div>
              <span className={"switch" + (cfg.footerLogo ? " on" : "")}><span className="switch__dot"></span></span>
            </div>
            {cfg.footerLogo && (
              <div className="fade-in" style={{ marginTop: 12 }}>
                <MediaPicker value={cfg.footerLogo} onPick={(src) => set("footerLogo", src)} lang={lang} />
                <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 10 }}>
                  <SizeSlider label={lang === "pt" ? "Tamanho" : "Size"} value={cfg.footerLogoSize} min={10} max={28} step={1} unit="px" onChange={(v) => set("footerLogoSize", v)} />
                  <SizeSlider label={lang === "pt" ? "Margem" : "Margin"} value={cfg.footerLogoGap} min={0} max={20} step={1} unit="px" onChange={(v) => set("footerLogoGap", v)} />
                </div>
              </div>
            )}
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Cor da marca" : "Brand color"}</div>
            <div className="swatches">
              {BRAND_COLORS.map((c) => <button key={c} className={"swatch" + (cfg.accent === c ? " on" : "")} style={{ background: c }} onClick={() => set("accent", c)}></button>)}
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Logo" : "Logo"}</div>
            <div className="kind-picker" style={{ gridTemplateColumns: "1fr 1fr 1fr" }}>
              {[["wordmark", lang === "pt" ? "Marca + nome" : "Mark + name", "ti-text-size"], ["mark", lang === "pt" ? "Só símbolo" : "Symbol only", "ti-square-rounded"], ["custom", lang === "pt" ? "Personalizado" : "Custom", "ti-photo"]].map(([k, lbl, ic]) => (
                <button key={k} className={"kind-opt" + (cfg.logo === k ? " on" : "")} style={{ flexDirection: "column", gap: 6, padding: "12px 6px", fontSize: 11.5, ...(cfg.logo === k ? { borderColor: cfg.accent } : null) }} onClick={() => set("logo", k)}>
                  <i className={"ti " + ic} style={{ fontSize: 18, color: cfg.logo === k ? cfg.accent : "var(--text-muted)" }}></i>{lbl}
                </button>
              ))}
            </div>
            {cfg.logo === "custom" && (
              <div className="fade-in" style={{ marginTop: 12 }}>
                <div className="form-label" style={{ marginBottom: 8, display: "flex", alignItems: "center", gap: 6 }}><i className="ti ti-photo" style={{ fontSize: 14, color: "var(--text-faint)" }}></i>{lang === "pt" ? "Escolher da biblioteca de mídia" : "Choose from media library"}</div>
                <MediaPicker value={cfg.logoUrl} onPick={(src) => set("logoUrl", src)} lang={lang} />
                <label className="form-field" style={{ marginTop: 12, marginBottom: 0 }}><span className="form-label" style={{ fontSize: 11, color: "var(--text-faint)" }}>{lang === "pt" ? "ou cole uma URL" : "or paste a URL"}</span>
                  <input className="form-input mono" value={cfg.logoUrl} onChange={(e) => set("logoUrl", e.target.value)} placeholder="assets/logos/portal.svg" /></label>
              </div>
            )}
            <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <SizeSlider label={lang === "pt" ? "Tamanho do logo" : "Logo size"} value={cfg.logoSize} min={12} max={32} step={1} unit="px" onChange={(v) => set("logoSize", v)} />
              <SizeSlider label={lang === "pt" ? "Margem do logo" : "Logo margin"} value={cfg.logoGap} min={0} max={20} step={1} unit="px" onChange={(v) => set("logoGap", v)} />
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Fundo & gradientes" : "Background & gradients"}</div>
            <Field label={lang === "pt" ? "Cabeçalho do portal" : "Portal header"}>
              <div className="grad-picker">
                {[["none", lang === "pt" ? "Liso" : "Flat"], ["soft", lang === "pt" ? "Suave" : "Soft"], ["bold", lang === "pt" ? "Vibrante" : "Bold"]].map(([k, lbl]) => (
                  <button key={k} className={"grad-opt" + (cfg.hero === k ? " on" : "")} onClick={() => set("hero", k)}>
                    <span className="grad-swatch" style={{ background: k === "none" ? "var(--surface-soft)" : k === "soft" ? `linear-gradient(135deg, color-mix(in srgb, ${cfg.accent} 22%, transparent), transparent)` : `linear-gradient(135deg, ${cfg.accent}, color-mix(in srgb, ${cfg.accent} 50%, #000))` }}></span>
                    {lbl}
                  </button>
                ))}
              </div>
            </Field>
            <Field label={lang === "pt" ? "Capa dos cards" : "Card covers"}>
              <div className="seg" style={{ width: "100%" }}>
                {[["tint", lang === "pt" ? "Tonalizado" : "Tinted"], ["gradient", lang === "pt" ? "Gradiente" : "Gradient"]].map(([k, lbl]) => (
                  <button key={k} className={cfg.cover === k ? "on" : ""} style={{ flex: 1, justifyContent: "center" }} onClick={() => set("cover", k)}>{lbl}</button>
                ))}
              </div>
            </Field>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Tema padrão" : "Default theme"}</div>
            <div className="seg seg--teal" style={{ width: "100%" }}>
              {[["light", lang === "pt" ? "Claro" : "Light", "ti-sun"], ["dark", lang === "pt" ? "Escuro" : "Dark", "ti-moon"], ["auto", "Auto", "ti-device-desktop"]].map(([k, lbl, ic]) => (
                <button key={k} className={cfg.theme === k ? "on" : ""} style={{ flex: 1, justifyContent: "center" }} onClick={() => set("theme", k)}><i className={"ti " + ic}></i>{lbl}</button>
              ))}
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Layout do catálogo" : "Catalog layout"}</div>
            <div className="kind-picker" style={{ gridTemplateColumns: "1fr 1fr 1fr" }}>
              {layouts.map((l) => (
                <button key={l.k} className={"kind-opt" + (cfg.layout === l.k ? " on" : "")} style={{ flexDirection: "column", gap: 6, padding: "12px 8px", ...(cfg.layout === l.k ? { borderColor: cfg.accent } : null) }} onClick={() => set("layout", l.k)}>
                  <i className={"ti " + l.icon} style={{ fontSize: 20, color: cfg.layout === l.k ? cfg.accent : "var(--text-muted)" }}></i>{l.label}
                </button>
              ))}
            </div>
            <div className="seg" style={{ marginTop: 12, width: "100%" }}>
              {[["comfortable", lang === "pt" ? "Confortável" : "Comfortable"], ["compact", lang === "pt" ? "Compacto" : "Compact"]].map(([k, lbl]) => (
                <button key={k} className={cfg.density === k ? "on" : ""} style={{ flex: 1, justifyContent: "center" }} onClick={() => set("density", k)}>{lbl}</button>
              ))}
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Seções visíveis" : "Visible sections"}</div>
            <Toggle label={lang === "pt" ? "Carrossel de destaques" : "Featured carousel"} on={cfg.featured} onClick={() => set("featured", !cfg.featured)} />
            <Toggle label={lang === "pt" ? "Barra de busca" : "Search bar"} on={cfg.showSearch} onClick={() => set("showSearch", !cfg.showSearch)} />
            <Toggle label={lang === "pt" ? "Filtros de acesso (livre/restrito)" : "Access filters (public/restricted)"} on={cfg.showAccess} onClick={() => set("showAccess", !cfg.showAccess)} last />
          </div>

          {/* SEO */}
          <div className="form-section">
            <div className="form-section__title"><i className="ti ti-search" style={{ fontSize: 13, marginRight: 6, verticalAlign: "-2px" }}></i>SEO</div>
            <label className="form-field"><span className="form-label">{lang === "pt" ? "Título (meta)" : "Meta title"}</span>
              <input className="form-input" value={cfg.seoTitle} onChange={(e) => set("seoTitle", e.target.value)} maxLength={70} />
              <div className={"char-count" + (cfg.seoTitle.length > 60 ? " over" : "")}>{cfg.seoTitle.length}/60</div>
            </label>
            <label className="form-field"><span className="form-label">{lang === "pt" ? "Descrição (meta)" : "Meta description"}</span>
              <textarea className="form-input" rows={3} value={cfg.seoDesc} onChange={(e) => set("seoDesc", e.target.value)} maxLength={200}></textarea>
              <div className={"char-count" + (cfg.seoDesc.length > 160 ? " over" : "")}>{cfg.seoDesc.length}/160</div>
            </label>
            <div className="form-label" style={{ marginBottom: 8 }}>{lang === "pt" ? "Imagem de compartilhamento (OG)" : "Share image (OG)"}</div>
            <MediaPicker value={cfg.ogImage} onPick={(src) => set("ogImage", src)} lang={lang} />
            <div className="form-label" style={{ margin: "16px 0 6px", fontSize: 11, color: "var(--text-faint)" }}>{lang === "pt" ? "Prévia no Google" : "Google preview"}</div>
            <div className="serp">
              <div className="serp__url"><i className="ti ti-world"></i>ruscker.app › portal</div>
              <div className="serp__title">{cfg.seoTitle || cfg.title}</div>
              <div className="serp__desc">{cfg.seoDesc}</div>
            </div>
          </div>

          {/* Analytics */}
          <div className="form-section">
            <div className="form-section__title"><i className="ti ti-chart-bar" style={{ fontSize: 13, marginRight: 6, verticalAlign: "-2px" }}></i>Analytics</div>
            <div className="provider-grid">
              {[["none", lang === "pt" ? "Nenhum" : "None", "ti-circle-off"], ["ga", "Google Analytics", "ti-brand-google"], ["plausible", "Plausible", "ti-chart-dots"], ["matomo", "Matomo", "ti-chart-area"]].map(([k, lbl, ic]) => (
                <button key={k} className={"kind-opt" + (cfg.analytics === k ? " on" : "")} style={{ padding: "10px 12px", fontSize: 12, ...(cfg.analytics === k ? { borderColor: cfg.accent } : null) }} onClick={() => set("analytics", k)}>
                  <i className={"ti " + ic} style={{ fontSize: 16, color: cfg.analytics === k ? cfg.accent : "var(--text-muted)" }}></i>{lbl}
                </button>
              ))}
            </div>
            {cfg.analytics !== "none" && (
              <div className="fade-in" style={{ marginTop: 14 }}>
                <label className="form-field"><span className="form-label">{cfg.analytics === "ga" ? "Measurement ID" : cfg.analytics === "plausible" ? (lang === "pt" ? "Domínio" : "Domain") : "Site ID"}</span>
                  <input className="form-input mono" value={cfg.analyticsId} onChange={(e) => set("analyticsId", e.target.value)} placeholder={cfg.analytics === "ga" ? "G-XXXXXXX" : cfg.analytics === "plausible" ? "ruscker.app" : "1"} /></label>
                <Toggle label={lang === "pt" ? "Respeitar Do Not Track" : "Respect Do Not Track"} on={cfg.respectDnt} onClick={() => set("respectDnt", !cfg.respectDnt)} last />
              </div>
            )}
          </div>

          {/* Custom CSS */}
          <div className="form-section">
            <div className="form-section__title"><i className="ti ti-brand-css3" style={{ fontSize: 13, marginRight: 6, verticalAlign: "-2px" }}></i>{lang === "pt" ? "CSS customizado" : "Custom CSS"}</div>
            <div className="toggle-row__hint" style={{ marginBottom: 10 }}>{lang === "pt" ? "Injetado no final do portal público. Use com cuidado." : "Injected at the end of the public portal. Use with care."}</div>
            <CodeEditor mode="css" minHeight={140}
              placeholder={lang === "pt" ? "/* CSS aplicado ao portal público */" : "/* CSS applied to the public portal */"}
              value={cfg.customCss} onChange={(v) => set("customCss", v)} />
          </div>

          {/* ── HTML Blocks ── */}
          <div className="form-section">
            <div className="form-section__title">
              <i className="ti ti-code" style={{ fontSize: 13, marginRight: 6, verticalAlign: "-2px" }}></i>
              {lang === "pt" ? "Blocos HTML" : "HTML blocks"}
            </div>
            <div className="toggle-row__hint" style={{ marginBottom: 14 }}>
              {lang === "pt"
                ? "Injete HTML em posições-chave do portal. Útil para banners, avisos, integrações e widgets."
                : "Inject HTML at key positions in the portal. Useful for banners, notices, integrations and widgets."}
            </div>
            {[
              { key: "htmlTop",           icon: "ti-layout-navbar",                  label: lang === "pt" ? "Abaixo do cabeçalho (antes dos destaques)" : "Below header (before featured)" },
              { key: "htmlAfterFeatured",  icon: "ti-layout-distribute-horizontal",   label: lang === "pt" ? "Após destaques (antes da busca)" : "After featured (before search)" },
              { key: "htmlBeforeFooter",   icon: "ti-layout-bottombar",               label: lang === "pt" ? "Antes do rodapé (após o catálogo)" : "Before footer (after catalog)" },
              { key: "htmlFooter",         icon: "ti-layout-bottombar-filled",        label: lang === "pt" ? "Dentro do rodapé" : "Inside footer" },
            ].map(({ key, icon, label }) => (
              <HtmlBlock key={key} label={label} icon={icon} val={cfg[key] || ""} lang={lang}
                onChange={(v) => set(key, v)} />
            ))}
          </div>
        </div>

        {/* ── live portal preview ── */}
        <div className="editor-preview">
          <div className="editor-preview__sticky" style={{ padding: 0, overflow: "hidden" }}>
            <div className="prev-toolbar"><span className="prev-dot"></span><span className="prev-dot"></span><span className="prev-dot"></span><span style={{ marginLeft: 8, fontSize: 11, color: "var(--text-faint)" }} className="mono">portal · {lang === "pt" ? "pré-visualização" : "preview"}</span></div>
            <PortalPreview cfg={cfg} t={t} featured={featured} />
          </div>
          <div className="preview-note">{lang === "pt" ? "Mudanças aparecem aqui na hora. Publique para aplicar." : "Changes appear here instantly. Publish to apply."}</div>
        </div>
      </div>
    </div>
  );
}

function Toggle({ label, on, onClick, last }) {
  return (
    <div className="toggle-row" style={last ? { borderBottom: 0 } : null} onClick={onClick}>
      <div className="toggle-row__label">{label}</div>
      <span className={"switch" + (on ? " on" : "")}><span className="switch__dot"></span></span>
    </div>
  );
}

function SizeSlider({ label, value, min, max, step, unit, onChange }) {
  return (
    <div className="res-row" style={{ gridTemplateColumns: "110px 1fr 56px", margin: 0 }}>
      <span className="res-row__k" style={{ fontSize: 12 }}>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value} className="rk-range" onChange={(e) => onChange(parseInt(e.target.value, 10))} />
      <span className="res-row__v tnum">{value}{unit}</span>
    </div>
  );
}

/* ── Mini featured carousel — chevron navigation, no scroll,
   mirrors the real Portal FeaturedCarousel ── */
function PvFeatCarousel({ apps, cfg, lang }) {
  const [page, setPage] = apS(0);
  const per = 3; // cards visible at once in the preview width
  const pages = Math.ceil(apps.length / per);
  const slice = apps.slice(page * per, page * per + per);
  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        <button className="pv-chev" disabled={page === 0} onClick={() => setPage(Math.max(0, page - 1))}><i className="ti ti-chevron-left" style={{ fontSize: 11 }}></i></button>
        <div style={{ flex: 1, display: "flex", gap: 6, overflow: "hidden" }}>
          {slice.map((a) => (
            <div key={a.id} className="pv-feat-card" style={{
              flex: "0 0 88px",
              background: `color-mix(in srgb, ${a.accent} 10%, var(--surface, #fff))`,
              borderColor: `color-mix(in srgb, ${a.accent} 22%, transparent)`,
            }}>
              <div className="pv-feat-card__logo" style={{ background: `color-mix(in srgb, ${a.accent} 18%, transparent)` }}>
                {a.logo ? <img src={a.logo} alt="" /> : <span style={{ color: a.accent, fontWeight: 600, fontSize: 11 }}>{a.mono}</span>}
              </div>
              <div className="pv-feat-card__name">{a.name}</div>
              <div className="pv-feat-card__desc">{a.desc[lang]}</div>
              <div className="pv-feat-card__cta" style={{ background: cfg.accent }}><i className="ti ti-arrow-right" style={{ fontSize: 10 }}></i></div>
            </div>
          ))}
        </div>
        <button className="pv-chev" disabled={page >= pages - 1} onClick={() => setPage(Math.min(pages - 1, page + 1))}><i className="ti ti-chevron-right" style={{ fontSize: 11 }}></i></button>
      </div>
      {pages > 1 && (
        <div style={{ display: "flex", gap: 4, justifyContent: "center", marginTop: 6 }}>
          {Array.from({ length: pages }).map((_, i) => (
            <span key={i} onClick={() => setPage(i)} style={{ cursor: "pointer", width: i === page ? 14 : 5, height: 5, borderRadius: 999, background: i === page ? cfg.accent : "color-mix(in srgb, currentColor 22%, transparent)", transition: "all .2s" }}></span>
          ))}
        </div>
      )}
    </div>
  );
}

/* ── Syntax-highlighted code editor (textarea + pre overlay) ──
   Supports CSS and HTML token coloring.                       */
/* ── Proper single-pass tokenizer + syntax highlighter ── */
function escHtml(s) { return s.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;"); }

function highlightCSS(raw) {
  let out = "", i = 0, n = raw.length;
  while (i < n) {
    // Comment /* ... */
    if (raw[i] === "/" && raw[i+1] === "*") {
      const e = raw.indexOf("*/", i+2); const end = e < 0 ? n : e+2;
      out += `<span class="tok-comment">${escHtml(raw.slice(i,end))}</span>`; i = end; continue;
    }
    // String
    if (raw[i] === '"' || raw[i] === "'") {
      const q = raw[i]; let j = i+1;
      while (j < n && raw[j] !== q) { if (raw[j] === "\\") j++; j++; }
      out += `<span class="tok-string">${escHtml(raw.slice(i,j+1))}</span>`; i = j+1; continue;
    }
    // At-rule
    if (raw[i] === "@") {
      const m = raw.slice(i).match(/^@[\w-]+/);
      if (m) { out += `<span class="tok-atrule">${escHtml(m[0])}</span>`; i += m[0].length; continue; }
    }
    // !important
    if (raw.slice(i,i+10) === "!important") {
      out += `<span class="tok-important">!important</span>`; i += 10; continue;
    }
    // Hex color
    if (raw[i] === "#") {
      const m = raw.slice(i).match(/^#[0-9a-fA-F]{3,8}\b/);
      if (m) { out += `<span class="tok-hexcolor">${escHtml(m[0])}</span>`; i += m[0].length; continue; }
    }
    // Property name (word before lone :)
    if (/[\w-]/.test(raw[i]) && i > 0) {
      // Peek: is this a property or selector?
      const m = raw.slice(i).match(/^([\w-]+)(\s*:(?!:))/);
      if (m) {
        const ctx = raw.slice(0, i).replace(/\/\*[\s\S]*?\*\//g,"");
        const inBlock = (ctx.match(/{/g)||[]).length > (ctx.match(/}/g)||[]).length;
        if (inBlock) {
          out += `<span class="tok-prop">${escHtml(m[1])}</span>`; i += m[1].length; continue;
        }
      }
    }
    // Number + optional unit
    if (/[\d.]/.test(raw[i]) && (i === 0 || !/[\w#]/.test(raw[i-1]))) {
      const m = raw.slice(i).match(/^-?\d*\.?\d+(px|em|rem|%|vh|vw|s|ms|deg|fr|ch|ex|vmin|vmax|svh)?\b/);
      if (m) { out += `<span class="tok-number">${escHtml(m[0])}</span>`; i += m[0].length; continue; }
    }
    // Selector hint: . or # at start of line
    if ((raw[i] === "." || raw[i] === "#") && (i === 0 || /[\n\r,{ ]/.test(raw[i-1]))) {
      const m = raw.slice(i).match(/^[.#][\w-]+/);
      if (m) { out += `<span class="tok-selector">${escHtml(m[0])}</span>`; i += m[0].length; continue; }
    }
    out += escHtml(raw[i++]);
  }
  return out + "\n";
}

function highlightHTML(raw) {
  let out = "", i = 0, n = raw.length;
  while (i < n) {
    if (raw.slice(i,i+4) === "<!--") {
      const e = raw.indexOf("-->", i+4); const end = e < 0 ? n : e+3;
      out += `<span class="tok-comment">${escHtml(raw.slice(i,end))}</span>`; i = end; continue;
    }
    if (raw[i] === "<") {
      const m = raw.slice(i).match(/^<\/?([\w:-]+)/);
      if (m) {
        out += `<span class="tok-tag">&lt;${m[1].startsWith("/")?"<span class=\"tok-tag\">":""}${escHtml(m[1])}${m[1].startsWith("/")?"</span>":""}</span>`;
        i += m[0].length; continue;
      }
    }
    if (raw[i] === ">" || (raw[i] === "/" && raw[i+1] === ">")) {
      const s = raw[i] === "/" ? "/>" : ">";
      out += `<span class="tok-tag">${escHtml(s)}</span>`; i += s.length; continue;
    }
    if (raw[i] === '"' || raw[i] === "'") {
      const q = raw[i]; let j = i+1;
      while (j < n && raw[j] !== q) j++;
      out += `<span class="tok-string">${escHtml(raw.slice(i,j+1))}</span>`; i = j+1; continue;
    }
    const am = raw.slice(i).match(/^[\w:-]+(?=\s*=)/);
    if (am) { out += `<span class="tok-attr">${escHtml(am[0])}</span>`; i += am[0].length; continue; }
    out += escHtml(raw[i++]);
  }
  return out + "\n";
}

function highlight(code, mode) {
  return mode === "html" ? highlightHTML(code) : highlightCSS(code);
}

function CodeEditor({ value, onChange, mode = "css", placeholder, minHeight = 120 }) {
  const ref = apR(null);
  const preRef = apR(null);
  const sync = () => {
    if (preRef.current && ref.current) {
      preRef.current.scrollTop = ref.current.scrollTop;
      preRef.current.scrollLeft = ref.current.scrollLeft;
    }
  };
  return (
    <div className="code-editor-wrap" style={{ minHeight }}>
      <pre ref={preRef} className="code-editor-pre" aria-hidden="true"
        dangerouslySetInnerHTML={{ __html: highlight(value || "", mode) }} />
      <textarea ref={ref} className="code-editor-ta" value={value || ""} placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)} onScroll={sync}
        spellCheck={false} autoCapitalize="off" autoCorrect="off" />
    </div>
  );
}
function HtmlBlock({ label, icon, val, lang, onChange }) {
  const [open, setOpen] = apS(false);
  const hasContent = val && val.trim().length > 0;
  return (
    <div className="html-block" style={{ marginBottom: 8 }}>
      <button className="html-block__head" onClick={() => setOpen((o) => !o)}>
        <span style={{ display: "flex", alignItems: "center", gap: 7, flex: 1, minWidth: 0 }}>
          <i className={"ti " + icon} style={{ fontSize: 13, color: "var(--text-faint)", flexShrink: 0 }}></i>
          <span style={{ fontSize: 12.5, fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{label}</span>
          {hasContent && <span className="html-block__dot"></span>}
        </span>
        <i className={"ti " + (open ? "ti-chevron-up" : "ti-chevron-down")} style={{ fontSize: 12, color: "var(--text-faint)", flexShrink: 0 }}></i>
      </button>
      {open && (
        <div className="html-block__body fade-in">
          <CodeEditor mode="html" minHeight={90}
            placeholder={lang === "pt" ? "<!-- HTML injetado aqui -->" : "<!-- HTML injected here -->"}
            value={val} onChange={onChange} />
          {hasContent && (
            <div className="html-block__preview">
              <span style={{ fontSize: 10, color: "var(--text-faint)", display: "block", marginBottom: 4 }}>{lang === "pt" ? "Pré-visualização:" : "Preview:"}</span>
              <div dangerouslySetInnerHTML={{ __html: val }} style={{ fontSize: 12, lineHeight: 1.5, border: "0.5px solid var(--border)", borderRadius: 6, padding: "8px 10px", background: "var(--surface-soft)", overflowX: "auto" }} />
            </div>
          )}
          <div style={{ display: "flex", gap: 6, marginTop: 8 }}>
            <button className="btn" style={{ fontSize: 11 }} onClick={() => onChange("")}>{lang === "pt" ? "Limpar" : "Clear"}</button>
            <button className="btn btn--primary" style={{ fontSize: 11 }} onClick={() => {}}>{lang === "pt" ? "Aplicar" : "Apply"}</button>
          </div>
        </div>
      )}
    </div>
  );
}
function PortalPreview({ cfg, t, featured }) {
  const lang = t._lang;
  const apps = window.RK.APPS.slice(0, 6);
  const featApps = window.RK.APPS.filter((a) => featured && featured.has(a.id));
  const featApp = featApps[0] || apps[0];
  const dark = cfg.theme === "dark";
  const gap = cfg.density === "compact" ? 6 : 9;
  const heroBg = cfg.hero === "none" ? "transparent"
    : cfg.hero === "soft" ? `linear-gradient(160deg, color-mix(in srgb, ${cfg.accent} 20%, transparent), transparent 80%)`
    : `linear-gradient(150deg, ${cfg.accent}, color-mix(in srgb, ${cfg.accent} 45%, #000))`;
  const headInk = cfg.hero === "bold" ? "#fff" : "inherit";
  const signinStyle = cfg.hero === "bold" ? { borderColor: "rgba(255,255,255,.6)", color: "#fff" } : { borderColor: cfg.accent, color: cfg.accent };
  return (
    <div className={"pv-root" + (dark ? " pv-dark" : "")} style={{ "--pv-accent": cfg.accent }}>
      <div className="pv-head" style={{ background: heroBg, color: headInk }}>
        <div className="pv-brand-col">
          <div className="pv-brand" style={{ gap: cfg.logoGap, alignItems: "center" }}>
            {cfg.logo === "custom" && cfg.logoUrl
              ? <img src={cfg.logoUrl} alt="" style={{ width: cfg.logoSize, height: cfg.logoSize, display: "block", flexShrink: 0 }} onError={(e) => { e.target.style.display = "none"; }} />
              : <img src="assets/ruscker-mark.svg" alt="" style={{ width: cfg.logoSize, height: cfg.logoSize, display: "block", flexShrink: 0 }} />}
            {cfg.logo !== "mark" && (
              <div>
                <b style={{ fontSize: 13, fontWeight: 700, letterSpacing: "-0.02em" }}>{cfg.title || "Portal"}</b>
                {cfg.tagline && <div className="pv-tag" style={{ marginTop: 1, fontSize: 8.5, opacity: cfg.hero === "bold" ? 0.85 : 0.6 }}>{cfg.tagline}</div>}
              </div>
            )}
          </div>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
          <span className="pv-icon-btn"><i className="ti ti-sun" style={{ fontSize: 11 }}></i></span>
          <span className="pv-icon-btn" style={{ fontSize: 9, fontWeight: 600 }}>{lang.toUpperCase()}</span>
          <span className="pv-icon-btn" style={signinStyle}><i className="ti ti-login-2" style={{ fontSize: 11 }}></i></span>
        </div>
      </div>
      <div className="pv-body">
        {cfg.featured && featApps.length > 0 && (
          <div className="pv-feat-wrap">
            <div className="pv-feat-label"><i className="ti ti-star-filled" style={{ color: cfg.accent }}></i>{lang === "pt" ? "Em destaque" : "Featured"} <span style={{ opacity: .55, fontWeight: 400 }}>· {featApps.length}</span></div>
            <PvFeatCarousel apps={featApps} cfg={cfg} lang={lang} />
          </div>
        )}

        {cfg.showSearch && (
          <div className="pv-search"><i className="ti ti-search"></i><span>{lang === "pt" ? "Buscar…" : "Search…"}</span></div>
        )}

        <div className="pv-chips">
          <span className="pv-chip on" style={{ background: cfg.accent }}>{lang === "pt" ? "Todos" : "All"}</span>
          <span className="pv-chip">Apps</span>
          <span className="pv-chip">API</span>
          {cfg.showAccess && <><span className="pv-chip pv-chip--ghost"><i className="ti ti-lock-open"></i></span><span className="pv-chip pv-chip--ghost"><i className="ti ti-lock"></i></span></>}
        </div>

        {cfg.layout === "list" ? (
          <div style={{ display: "flex", flexDirection: "column", gap }}>
            {apps.map((a) => (
              <div className="pv-row" key={a.id}>
                <span className="pv-logo" style={{ background: `color-mix(in srgb, ${a.accent} 15%, transparent)` }}>{a.logo ? <img src={a.logo} alt="" /> : a.mono}</span>
                <div style={{ flex: 1, minWidth: 0 }}><div className="pv-row-name">{a.name}</div><div className="pv-row-desc">{a.desc[lang]}</div></div>
              </div>
            ))}
          </div>
        ) : cfg.layout === "sections" ? (
          ["Apps", "API"].map((sec, si) => (
            <div key={sec} style={{ marginBottom: 10 }}>
              <div className="pv-sec-head">{sec}</div>
              <div className="pv-rail">{apps.slice(si * 3, si * 3 + 3).map((a) => <MiniCard key={a.id} a={a} cfg={cfg} />)}</div>
            </div>
          ))
        ) : (
          <div className="pv-grid" style={{ gap }}>{apps.map((a) => <MiniCard key={a.id} a={a} cfg={cfg} />)}</div>
        )}
      </div>
      <div className="pv-foot">
        {cfg.footerLogo && <img className="pv-foot-logo" src={cfg.footerLogo} alt="" style={{ width: cfg.footerLogoSize, height: cfg.footerLogoSize, marginRight: cfg.footerLogoGap }} onError={(e) => { e.target.style.display = "none"; }} />}
        <span>{cfg.footer}</span>
      </div>
    </div>
  );
}

function MiniCard({ a, cfg }) {
  const cover = cfg.cover === "gradient"
    ? `linear-gradient(140deg, color-mix(in srgb, ${a.accent} 30%, transparent), color-mix(in srgb, ${a.accent} 6%, transparent))`
    : `color-mix(in srgb, ${a.accent} 12%, transparent)`;
  return (
    <div className="pv-card">
      <div className="pv-card-cover" style={{ background: cover }}>
        {a.logo ? <img src={a.logo} alt="" /> : <span style={{ color: a.accent, fontWeight: 600 }}>{a.mono}</span>}
      </div>
      <div className="pv-card-name">{a.name}</div>
    </div>
  );
}

window.RKAppearance = Appearance;
window.RKMedia = { mediaAssets, MediaPicker };
