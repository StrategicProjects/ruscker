/* global React, window */
// ── Admin forms: App editor (live preview) + Import YAML ──
const { useState: fS, useMemo: fM } = React;
const AF = window.RKC;

const ACCENTS = ["#447099", "#f37726", "#ff4b4b", "#0d76bf", "#059487", "#26547c", "#0f6e56", "#ef476f"];
const RUNTIMES = {
  pt: { "r": "R", "python": "Python", "quarto": "Quarto", "static": "Estático" },
  en: { "r": "R", "python": "Python", "quarto": "Quarto", "static": "Static" },
};

/* ═══════════════════════════════ APP EDITOR ════════════════════ */
function AppEditor({ t, app, isNew, onClose, push, appAccess, setAccess }) {
  const lang = t._lang;
  const [form, setForm] = fS(() => ({
    id: isNew ? "" : app.id,
    name: isNew ? "" : app.name,
    kind: isNew ? "app" : app.kind,
    descPt: isNew ? "" : (app.desc.pt || ""),
    descEn: isNew ? "" : (app.desc.en || ""),
    subject: isNew ? "Documentação" : app.subject,
    locked: isNew ? true : app.locked,
    replicas: isNew ? 1 : app.replicas,
    accessGroups: isNew ? [] : (appAccess ? (appAccess.get(app.id) ?? app.accessGroups ?? []) : (app.accessGroups || [])),
    accent: isNew ? ACCENTS[6] : app.accent,
    mono: isNew ? "" : app.mono,
    logo: isNew ? null : app.logo,
    featured: isNew ? false : !!app.featured,
    // advanced
    runtime: isNew ? "r" : (app.kind === "api" ? "python" : "r"),
    cmd: "",
    workdir: "/srv/app",
    port: 3838,
    healthPath: "/",
    timeout: 15,
    maxSessions: 10,
    cpu: 1,
    mem: 512,
    autoscale: false,
    minR: 1,
    maxR: 4,
    env: [{ key: "", value: "" }],
  }));
  const [showAdv, setShowAdv] = fS(!isNew);
  const set = (k, v) => setForm((f) => ({ ...f, [k]: v }));

  // live preview app object
  const preview = {
    ...form,
    name: form.name || (lang === "pt" ? "Nome do app" : "App name"),
    mono: form.mono || (form.name ? form.name.slice(0, 2) : "Ap"),
    desc: { pt: form.descPt || (lang === "pt" ? "Descrição curta do aplicativo." : ""), en: form.descEn || "Short app description." },
    status: "none", updated: "05/06",
  };

  const KINDS = [
    { k: "app", label: lang === "pt" ? "Aplicação" : "Application", icon: "ti-browser" },
    { k: "api", label: "API", icon: "ti-api" },
    { k: "package", label: lang === "pt" ? "Pacote" : "Package", icon: "ti-package" },
    { k: "report", label: lang === "pt" ? "Relatório" : "Report", icon: "ti-file-text" },
  ];

  const valid = form.id.trim() && form.name.trim();
  const setEnv = (i, k, v) => setForm((f) => { const env = f.env.map((e, j) => j === i ? { ...e, [k]: v } : e); return { ...f, env }; });
  const addEnv = () => setForm((f) => ({ ...f, env: [...f.env, { key: "", value: "" }] }));
  const rmEnv = (i) => setForm((f) => ({ ...f, env: f.env.filter((_, j) => j !== i).length ? f.env.filter((_, j) => j !== i) : [{ key: "", value: "" }] }));
  const toggleGroup = (g) => set("accessGroups", form.accessGroups.includes(g) ? form.accessGroups.filter((x) => x !== g) : [...form.accessGroups, g]);
  const GROUP_COLOR = { admin: "#ef476f", editor: "#06b6d4", viewer: "#26547c" };
  const ALL_GROUPS = ["admin","editor","viewer"];
  const save = () => { if (!valid) return; if (setAccess && !isNew) setAccess(form.id, form.accessGroups); push((isNew ? (lang === "pt" ? "App criado · " : "App created · ") : (lang === "pt" ? "Alterações salvas · " : "Saved · ")) + (form.name || form.id), "ti-check"); onClose(); };

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div className="crumbs">
        <a onClick={onClose}>Apps</a><i className="ti ti-chevron-right"></i>
        <span style={{ color: "var(--text)" }}>{isNew ? (lang === "pt" ? "Novo app" : "New app") : form.name}</span>
      </div>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 22 }}>
        <div className="page-title">{isNew ? (lang === "pt" ? "Novo aplicativo" : "New application") : (lang === "pt" ? "Editar aplicativo" : "Edit application")}</div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn" onClick={onClose}>{lang === "pt" ? "Cancelar" : "Cancel"}</button>
          <button className={"btn btn--primary" + (valid ? "" : " is-disabled")} onClick={save}><i className="ti ti-device-floppy"></i>{lang === "pt" ? "Salvar" : "Save"}</button>
        </div>
      </div>

      <div className="editor-grid">
        {/* form */}
        <div className="editor-form">
          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Identidade" : "Identity"}</div>
            <div className="form-row two">
              <Field label="ID" hint={lang === "pt" ? "slug único na URL" : "unique URL slug"}>
                <input className="form-input mono" value={form.id} onChange={(e) => set("id", e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "-"))} placeholder="meu-app" />
              </Field>
              <Field label={lang === "pt" ? "Nome" : "Name"}>
                <input className="form-input" value={form.name} onChange={(e) => set("name", e.target.value)} placeholder={lang === "pt" ? "Meu App" : "My App"} />
              </Field>
            </div>
            <Field label={lang === "pt" ? "Assunto / categoria" : "Subject / category"}>
              <input className="form-input" value={form.subject} onChange={(e) => set("subject", e.target.value)} />
            </Field>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Tipo" : "Kind"}</div>
            <div className="kind-picker">
              {KINDS.map((kk) => (
                <button key={kk.k} className={"kind-opt" + (form.kind === kk.k ? " on" : "")} onClick={() => set("kind", kk.k)} style={form.kind === kk.k ? { borderColor: window.RK.KIND[kk.k].color } : null}>
                  <i className={"ti " + kk.icon} style={{ color: window.RK.KIND[kk.k].color }}></i>{kk.label}
                </button>
              ))}
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Descrição" : "Description"}</div>
            <Field label="Português">
              <textarea className="form-input" rows={2} value={form.descPt} onChange={(e) => set("descPt", e.target.value)} placeholder={lang === "pt" ? "Descrição curta…" : "Short description…"}></textarea>
            </Field>
            <Field label="English">
              <textarea className="form-input" rows={2} value={form.descEn} onChange={(e) => set("descEn", e.target.value)} placeholder="Short description…"></textarea>
            </Field>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Aparência" : "Appearance"}</div>
            <Field label={lang === "pt" ? "Cor de destaque" : "Accent color"}>
              <div className="swatches">
                {ACCENTS.map((c) => <button key={c} className={"swatch" + (form.accent === c ? " on" : "")} style={{ background: c }} onClick={() => set("accent", c)}></button>)}
              </div>
            </Field>
            <div className="form-label" style={{ marginBottom: 8, display: "flex", alignItems: "center", gap: 6 }}><i className="ti ti-photo" style={{ fontSize: 14, color: "var(--text-faint)" }}></i>{lang === "pt" ? "Logo do app (da mídia)" : "App logo (from media)"}</div>
            <window.RKMedia.MediaPicker value={form.logo} onPick={(src) => set("logo", form.logo === src ? null : src)} lang={lang} />
            <div className="form-row two" style={{ marginTop: 14 }}>
              <Field label={lang === "pt" ? "Monograma (fallback)" : "Monogram (fallback)"}>
                <input className="form-input" maxLength={2} value={form.mono} onChange={(e) => set("mono", e.target.value)} placeholder="Ap" />
              </Field>
              <Field label={lang === "pt" ? "ou URL do logo" : "or Logo URL"}>
                <input className="form-input mono" value={form.logo || ""} onChange={(e) => set("logo", e.target.value || null)} placeholder="assets/logos/…" />
              </Field>
            </div>
          </div>

          <div className="form-section">
            <div className="form-section__title">{lang === "pt" ? "Acesso & escala" : "Access & scale"}</div>
            <div className="toggle-row" onClick={() => set("locked", !form.locked)}>
              <div><div className="toggle-row__label">{lang === "pt" ? "Acesso restrito" : "Restricted access"}</div><div className="toggle-row__hint">{lang === "pt" ? "Exige login para abrir" : "Requires sign-in to open"}</div></div>
              <span className={"switch" + (form.locked ? " on" : "")}><span className="switch__dot"></span></span>
            </div>

            {/* Group access picker */}
            <div style={{ paddingTop: 14 }}>
              <div className="form-label" style={{ marginBottom: 9 }}>
                <i className="ti ti-shield-lock" style={{ fontSize: 14, marginRight: 6, verticalAlign: "-2px", color: "var(--text-faint)" }}></i>
                {lang === "pt" ? "Grupos com acesso" : "Groups with access"}
                <span style={{ marginLeft: 8, fontSize: 11, color: "var(--text-faint)", fontWeight: 400 }}>{lang === "pt" ? "— vazio = público (todos)" : "— empty = public (everyone)"}</span>
              </div>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
                <button className={"group-toggle" + (form.accessGroups.length === 0 ? " on" : "")}
                  style={form.accessGroups.length === 0 ? { borderColor: "var(--teal-600)", background: "color-mix(in srgb, var(--teal-600) 14%, transparent)", color: "var(--teal-600)" } : null}
                  onClick={() => set("accessGroups", [])}>
                  <i className={"ti " + (form.accessGroups.length === 0 ? "ti-check" : "ti-world")} style={{ fontSize: 13 }}></i>
                  {lang === "pt" ? "Público" : "Public"}
                </button>
                {ALL_GROUPS.map((g) => (
                  <button key={g}
                    className={"group-toggle" + (form.accessGroups.includes(g) ? " on" : "")}
                    disabled={form.accessGroups.length === 0}
                    style={{
                      ...(form.accessGroups.includes(g) ? { borderColor: GROUP_COLOR[g], background: `color-mix(in srgb, ${GROUP_COLOR[g]} 14%, transparent)`, color: GROUP_COLOR[g] } : null),
                      ...(form.accessGroups.length === 0 ? { opacity: 0.32, cursor: "not-allowed" } : null),
                    }}
                    onClick={() => { if (form.accessGroups.length > 0) toggleGroup(g); }}>
                    <i className={"ti " + (form.accessGroups.includes(g) ? "ti-check" : "ti-plus")} style={{ fontSize: 12 }}></i>
                    {g === "admin" ? (lang === "pt" ? "Administrador" : "Administrator") : g === "editor" ? "Editor" : lang === "pt" ? "Visualizador" : "Viewer"}
                  </button>
                ))}
              </div>
              {form.accessGroups.length > 0 && (
                <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 8 }}>
                  <i className="ti ti-info-circle" style={{ fontSize: 12, marginRight: 4 }}></i>
                  {lang === "pt"
                    ? `Visível apenas para usuários dos grupos: ${form.accessGroups.join(", ")}`
                    : `Visible only to users in groups: ${form.accessGroups.join(", ")}`}
                </div>
              )}
            </div>

            <div className="toggle-row" onClick={() => set("featured", !form.featured)}>
              <div><div className="toggle-row__label">{lang === "pt" ? "Em destaque" : "Featured"}</div><div className="toggle-row__hint">{lang === "pt" ? "Aparece no carrossel do portal" : "Shows in the portal carousel"}</div></div>
              <span className={"switch" + (form.featured ? " on" : "")}><span className="switch__dot"></span></span>
            </div>
            <Field label={lang === "pt" ? "Réplicas iniciais" : "Initial replicas"}>
              <div className="stepper">
                <button onClick={() => set("replicas", Math.max(1, form.replicas - 1))}><i className="ti ti-minus"></i></button>
                <span className="tnum">{form.replicas}</span>
                <button onClick={() => set("replicas", Math.min(10, form.replicas + 1))}><i className="ti ti-plus"></i></button>
              </div>
            </Field>
          </div>

          {/* ── Advanced ── */}
          <div className={"form-section adv-section" + (showAdv ? " is-open" : "")}>
            <button className="adv-head" onClick={() => setShowAdv((s) => !s)}>
              <span className="form-section__title" style={{ margin: 0 }}><i className="ti ti-adjustments" style={{ fontSize: 14, marginRight: 6, verticalAlign: "-2px" }}></i>{lang === "pt" ? "Avançado" : "Advanced"}</span>
              <span className="adv-head__hint">{lang === "pt" ? "Runtime, recursos, ambiente, autoscaling" : "Runtime, resources, env, autoscaling"}</span>
              <i className="ti ti-chevron-down adv-chev"></i>
            </button>

            {showAdv && (
              <div className="adv-body fade-in">
                <div className="form-row two">
                  <Field label="Runtime">
                    <div className="kind-picker" style={{ gridTemplateColumns: "1fr 1fr" }}>
                      {Object.keys(RUNTIMES[lang]).map((rk) => (
                        <button key={rk} className={"kind-opt" + (form.runtime === rk ? " on" : "")} style={{ padding: "8px 10px" }} onClick={() => set("runtime", rk)}>{RUNTIMES[lang][rk]}</button>
                      ))}
                    </div>
                  </Field>
                  <Field label={lang === "pt" ? "Porta interna" : "Internal port"}>
                    <input className="form-input mono" value={form.port} onChange={(e) => set("port", e.target.value.replace(/\D/g, "").slice(0, 5))} />
                  </Field>
                </div>

                <Field label={lang === "pt" ? "Comando de inicialização" : "Start command"} hint={lang === "pt" ? "opcional" : "optional"}>
                  <input className="form-input mono" value={form.cmd} onChange={(e) => set("cmd", e.target.value)} placeholder={form.runtime === "python" ? "uvicorn app:app --port 3838" : "R -e 'shiny::runApp()'"} />
                </Field>
                <div className="form-row two">
                  <Field label={lang === "pt" ? "Diretório de trabalho" : "Working directory"}>
                    <input className="form-input mono" value={form.workdir} onChange={(e) => set("workdir", e.target.value)} />
                  </Field>
                  <Field label="Health check">
                    <input className="form-input mono" value={form.healthPath} onChange={(e) => set("healthPath", e.target.value)} placeholder="/health" />
                  </Field>
                </div>

                {/* resources */}
                <div className="form-label" style={{ marginBottom: 10 }}>{lang === "pt" ? "Recursos por réplica" : "Resources per replica"}</div>
                <div className="res-row">
                  <span className="res-row__k"><i className="ti ti-cpu"></i> CPU</span>
                  <input type="range" min="0.5" max="8" step="0.5" value={form.cpu} onChange={(e) => set("cpu", parseFloat(e.target.value))} className="rk-range" />
                  <span className="res-row__v tnum">{form.cpu} {lang === "pt" ? "núcleos" : "cores"}</span>
                </div>
                <div className="res-row">
                  <span className="res-row__k"><i className="ti ti-database"></i> {lang === "pt" ? "Memória" : "Memory"}</span>
                  <input type="range" min="256" max="8192" step="256" value={form.mem} onChange={(e) => set("mem", parseInt(e.target.value, 10))} className="rk-range" />
                  <span className="res-row__v tnum">{form.mem >= 1024 ? (form.mem / 1024).toFixed(form.mem % 1024 ? 1 : 0) + " GB" : form.mem + " MB"}</span>
                </div>
                <div className="form-row two" style={{ marginTop: 4 }}>
                  <Field label={lang === "pt" ? "Timeout de sessão (min)" : "Session timeout (min)"}>
                    <input className="form-input mono" value={form.timeout} onChange={(e) => set("timeout", e.target.value.replace(/\D/g, "").slice(0, 4))} />
                  </Field>
                  <Field label={lang === "pt" ? "Máx. sessões / réplica" : "Max sessions / replica"}>
                    <input className="form-input mono" value={form.maxSessions} onChange={(e) => set("maxSessions", e.target.value.replace(/\D/g, "").slice(0, 4))} />
                  </Field>
                </div>

                {/* env vars */}
                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", margin: "6px 0 8px" }}>
                  <span className="form-label">{lang === "pt" ? "Variáveis de ambiente" : "Environment variables"}</span>
                  <button className="mini-add" onClick={addEnv}><i className="ti ti-plus"></i>{lang === "pt" ? "Adicionar" : "Add"}</button>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
                  {form.env.map((e, i) => (
                    <div className="env-row" key={i}>
                      <input className="form-input mono" value={e.key} onChange={(ev) => setEnv(i, "key", ev.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "_"))} placeholder="CHAVE" />
                      <span className="env-eq">=</span>
                      <input className="form-input mono" value={e.value} onChange={(ev) => setEnv(i, "value", ev.target.value)} placeholder="valor" />
                      <button className="star-toggle" style={{ width: 30, height: 30 }} onClick={() => rmEnv(i)}><i className="ti ti-x" style={{ fontSize: 14 }}></i></button>
                    </div>
                  ))}
                </div>

                {/* autoscaling */}
                <div className="toggle-row" style={{ marginTop: 14 }} onClick={() => set("autoscale", !form.autoscale)}>
                  <div><div className="toggle-row__label">{lang === "pt" ? "Autoescala" : "Autoscaling"}</div><div className="toggle-row__hint">{lang === "pt" ? "Ajusta réplicas conforme a demanda" : "Scales replicas with demand"}</div></div>
                  <span className={"switch" + (form.autoscale ? " on" : "")}><span className="switch__dot"></span></span>
                </div>
                {form.autoscale && (
                  <div className="form-row two fade-in" style={{ marginTop: 12 }}>
                    <Field label={lang === "pt" ? "Mín. réplicas" : "Min replicas"}>
                      <div className="stepper">
                        <button onClick={() => set("minR", Math.max(1, form.minR - 1))}><i className="ti ti-minus"></i></button>
                        <span className="tnum">{form.minR}</span>
                        <button onClick={() => set("minR", Math.min(form.maxR, form.minR + 1))}><i className="ti ti-plus"></i></button>
                      </div>
                    </Field>
                    <Field label={lang === "pt" ? "Máx. réplicas" : "Max replicas"}>
                      <div className="stepper">
                        <button onClick={() => set("maxR", Math.max(form.minR, form.maxR - 1))}><i className="ti ti-minus"></i></button>
                        <span className="tnum">{form.maxR}</span>
                        <button onClick={() => set("maxR", Math.min(20, form.maxR + 1))}><i className="ti ti-plus"></i></button>
                      </div>
                    </Field>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        {/* live preview */}
        <div className="editor-preview">
          <div className="editor-preview__sticky">
            <div className="form-section__title" style={{ marginBottom: 12 }}><i className="ti ti-eye" style={{ fontSize: 14, marginRight: 6, verticalAlign: "-2px" }}></i>{lang === "pt" ? "Pré-visualização" : "Live preview"}</div>
            <div style={{ display: "flex", justifyContent: "center" }}>
              <AF.AppCard app={preview} t={t} onOpen={() => {}} />
            </div>
            <div className="spec-summary">
              <div className="spec-summary__row"><i className="ti ti-cpu"></i><span>{form.cpu} {lang === "pt" ? "núcleos" : "cores"} · {form.mem >= 1024 ? (form.mem / 1024).toFixed(form.mem % 1024 ? 1 : 0) + " GB" : form.mem + " MB"}</span></div>
              <div className="spec-summary__row"><i className="ti ti-stack-2"></i><span>{form.autoscale ? `${form.minR}–${form.maxR} ${lang === "pt" ? "réplicas (auto)" : "replicas (auto)"}` : `${form.replicas} ${lang === "pt" ? "réplicas" : "replicas"}`}</span></div>
              <div className="spec-summary__row"><i className="ti ti-clock"></i><span>{lang === "pt" ? "timeout" : "timeout"} {form.timeout} min · {form.maxSessions} {lang === "pt" ? "sessões" : "sessions"}</span></div>
            </div>
            <div className="preview-note">{lang === "pt" ? "É exatamente assim que o card aparece no portal." : "This is exactly how the card appears on the portal."}</div>
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({ label, hint, children }) {
  return (
    <label className="form-field">
      <span className="form-label">{label}{hint && <span className="form-hint"> · {hint}</span>}</span>
      {children}
    </label>
  );
}

/* ═══════════════════════════════ IMPORT YAML ══════════════════ */
const SAMPLE_YAML = `apps:
  - id: penguins-eda
    name: Penguins EDA
    kind: app
    subject: Ciência de dados
    locked: true
    replicas: 3
    description:
      pt: Análise exploratória do dataset de pinguins.
      en: Exploratory analysis of the penguins dataset.

  - id: forecast-api
    name: Forecast API
    kind: api
    locked: false
    replicas: 2
    description:
      pt: Previsão de séries temporais via REST.
      en: Time-series forecasting over REST.

  - id: sales-dashboard
    name: Sales Dashboard
    kind: app
    locked: true
    replicas: 4
    description:
      pt: Painel de vendas em tempo real.
      en: Real-time sales dashboard.

  - id: docs-quarto
    name: Docs (Quarto)
    kind: report
    locked: false
    replicas: 1
    description:
      pt: Documentação técnica publicada com Quarto.
      en: Technical docs published with Quarto.

  - id: geo-utils
    name: geo-utils
    kind: package
    locked: false
    replicas: 1
    description:
      pt: Pacote utilitário de geoprocessamento.
      en: Geoprocessing utility package.`;

function ImportYaml({ t, onClose, push }) {
  const lang = t._lang;
  const [text, setText] = fS(SAMPLE_YAML);
  const [sel, setSel] = fS(null); // Set of selected ids; null = all
  const APPS = window.RK.APPS;
  const existingIds = fM(() => new Set(APPS.map((a) => a.id)), []);

  // ultra-light parse: split on "- id:" and pull a few fields for preview
  const parsed = fM(() => {
    const blocks = text.split(/\n\s*-\s+id:/).slice(1);
    return blocks.map((b) => {
      const id = b.trim().split(/\s|\n/)[0] || "?";
      const name = (b.match(/name:\s*(.+)/) || [])[1] || id;
      const kind = (b.match(/kind:\s*(.+)/) || [])[1] || "app";
      const locked = /locked:\s*true/.test(b);
      const replicas = ((b.match(/replicas:\s*(\d+)/) || [])[1]) || "1";
      return { id: id.trim(), name: name.trim(), kind: kind.trim(), locked, replicas };
    });
  }, [text]);

  const ids = parsed.map((p) => p.id);
  const selected = sel === null ? new Set(ids) : new Set([...sel].filter((id) => ids.includes(id)));
  const allOn = ids.length > 0 && ids.every((id) => selected.has(id));
  const someOn = ids.some((id) => selected.has(id)) && !allOn;
  const toggle = (id) => { const n = new Set(selected); n.has(id) ? n.delete(id) : n.add(id); setSel(n); };
  const toggleAll = () => setSel(allOn ? new Set() : new Set(ids));
  const count = selected.size;
  const valid = count > 0 && parsed.every((p) => p.id && p.id !== "?");
  const nNew = parsed.filter((p) => selected.has(p.id) && !existingIds.has(p.id)).length;
  const nUpd = parsed.filter((p) => selected.has(p.id) && existingIds.has(p.id)).length;

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div className="crumbs">
        <a onClick={onClose}>Apps</a><i className="ti ti-chevron-right"></i>
        <span style={{ color: "var(--text)" }}>{lang === "pt" ? "Importar YAML" : "Import YAML"}</span>
      </div>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 22 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Importar apps de YAML" : "Import apps from YAML"}</div>
          <div className="page-sub">{lang === "pt" ? "Cole a definição ou arraste um arquivo .yml" : "Paste the definition or drop a .yml file"}</div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn" onClick={onClose}>{lang === "pt" ? "Cancelar" : "Cancel"}</button>
          <button className={"btn btn--primary" + (valid ? "" : " is-disabled")} onClick={() => { if (valid) { push((lang === "pt" ? "Importados " : "Imported ") + count + (lang === "pt" ? " apps" : " apps"), "ti-check"); onClose(); } }}>
            <i className="ti ti-file-import"></i>{lang === "pt" ? `Importar ${count}` : `Import ${count}`}{count !== parsed.length && parsed.length > 0 ? `/${parsed.length}` : ""}
          </button>
        </div>
      </div>

      <div className="editor-grid">
        <div>
          <div className="yaml-head">
            <span><i className="ti ti-file-code" style={{ fontSize: 14, marginRight: 6, verticalAlign: "-2px" }}></i>apps.yml</span>
            <button className="star-toggle" title={lang === "pt" ? "Restaurar exemplo" : "Reset sample"} onClick={() => { setText(SAMPLE_YAML); setSel(null); }} style={{ width: 26, height: 26 }}><i className="ti ti-rotate" style={{ fontSize: 14 }}></i></button>
          </div>
          <textarea className="yaml-editor mono" value={text} onChange={(e) => setText(e.target.value)} spellCheck={false}></textarea>
        </div>

        <div className="editor-preview">
          <div className="editor-preview__sticky">
            <div className="import-head">
              <label className="sel-all" onClick={(e) => { e.preventDefault(); toggleAll(); }}>
                <span className={"rk-check" + (allOn ? " on" : someOn ? " mixed" : "")}><i className={"ti " + (allOn ? "ti-check" : someOn ? "ti-minus" : "")}></i></span>
                <span style={{ fontSize: 12, fontWeight: 500 }}>{lang === "pt" ? "Selecionar todos" : "Select all"}</span>
              </label>
              <span className="tnum faint" style={{ fontSize: 11 }}>{count}/{parsed.length}</span>
            </div>
            {parsed.length === 0 ? (
              <div className="preview-note">{lang === "pt" ? "Nenhum app reconhecido ainda." : "No app recognized yet."}</div>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {parsed.map((p) => {
                  const on = selected.has(p.id);
                  const exists = existingIds.has(p.id);
                  return (
                    <div className={"parsed-row" + (on ? "" : " is-off")} key={p.id} onClick={() => toggle(p.id)} style={{ cursor: "pointer" }}>
                      <span className={"rk-check" + (on ? " on" : "")}><i className="ti ti-check"></i></span>
                      <span className="parsed-row__ico" style={{ background: "color-mix(in srgb, " + (window.RK.KIND[p.kind] ? window.RK.KIND[p.kind].color : "#888") + " 16%, var(--surface))" }}>
                        <i className={"ti " + (p.kind === "api" ? "ti-api" : p.kind === "package" ? "ti-package" : p.kind === "report" ? "ti-file-text" : "ti-browser")}></i>
                      </span>
                      <div style={{ minWidth: 0, flex: 1 }}>
                        <div style={{ fontSize: 13, fontWeight: 500, display: "flex", alignItems: "center", gap: 6 }}>{p.name}
                          <span className="import-tag" style={exists ? { background: "rgba(245,158,11,.16)", color: "#92600a" } : { background: "rgba(16,185,129,.16)", color: "#047857" }}>{exists ? (lang === "pt" ? "atualiza" : "update") : (lang === "pt" ? "novo" : "new")}</span>
                        </div>
                        <div className="mono faint" style={{ fontSize: 10.5 }}>{p.id} · {p.replicas} {lang === "pt" ? "réplicas" : "replicas"}</div>
                      </div>
                      <i className={"ti " + (p.locked ? "ti-lock" : "ti-lock-open")} style={{ fontSize: 14, color: p.locked ? "var(--lock)" : "var(--lock-open)" }}></i>
                    </div>
                  );
                })}
              </div>
            )}
            {count > 0 && (
              <div className="import-summary">
                {nNew > 0 && <span><b className="tnum">{nNew}</b> {lang === "pt" ? "novos" : "new"}</span>}
                {nNew > 0 && nUpd > 0 && <span>·</span>}
                {nUpd > 0 && <span><b className="tnum">{nUpd}</b> {lang === "pt" ? "atualizados" : "updated"}</span>}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

window.RKForms = { AppEditor, ImportYaml };
