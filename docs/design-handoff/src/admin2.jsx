/* global React, window */
// ── Admin pages: Disk · Credentials · Logs (refined style) ──
const { useState: a2S, useMemo: a2M, useEffect: a2E, useRef: a2R } = React;
const A2 = window.RKC;

const fmtGB = (mb) => (mb >= 1024 ? (mb / 1024).toFixed(1) + " GB" : Math.round(mb) + " MB");

/* ══════════════════════════════════════ DISK ══════════════════ */
const SEED_IMAGES = [
  { ref: "ruscker/shiny", tag: "4.3", mb: 1240, usedBy: 3 },
  { ref: "ruscker/shiny", tag: "4.2", mb: 1180, usedBy: 0 },
  { ref: "ruscker/python", tag: "3.12", mb: 980, usedBy: 4 },
  { ref: "ruscker/jupyter", tag: "7", mb: 1410, usedBy: 2 },
  { ref: "ruscker/rstudio", tag: "2024.04", mb: 1620, usedBy: 1 },
  { ref: "ruscker/quarto", tag: "1.3", mb: 720, usedBy: 0 },
  { ref: "ruscker/streamlit", tag: "1.32", mb: 640, usedBy: 1 },
  { ref: "<none>", tag: "<none>", mb: 410, usedBy: 0, dangling: true },
];
const SEED_CONTAINERS = [
  { cid: "a1c4f29e", app: "Shiny", state: "running", mb: 84, created: "2 h" },
  { cid: "9f2b7d10", app: "Jupyter", state: "running", mb: 132, created: "5 h" },
  { cid: "77de03a2", app: "Streamlit", state: "running", mb: 96, created: "1 d" },
  { cid: "3c0a55b1", app: "Shiny", state: "exited", mb: 78, created: "3 d" },
  { cid: "b8e1f4c7", app: "RStudio", state: "stopped", mb: 210, created: "6 d" },
  { cid: "5d92a0fe", app: "Voilà", state: "exited", mb: 64, created: "8 d" },
];

function AdminDisk({ t, loading, push }) {
  const lang = t._lang;
  const TOTAL = 51200; // 50 GB
  const [images, setImages] = a2S(SEED_IMAGES);
  const [containers, setContainers] = a2S(SEED_CONTAINERS);

  const STATIC = [
    { id: "cache", label: lang === "pt" ? "Cache de sessões" : "Session cache", mb: 6420, color: "#3b82f6", icon: "ti-bolt" },
    { id: "logs", label: "Logs", mb: 1890, color: "#f59e0b", icon: "ti-terminal-2" },
    { id: "backups", label: "Backups", mb: 2110, color: "#8b5cf6", icon: "ti-database-export" },
  ];
  const imagesMb = images.reduce((s, i) => s + i.mb, 0);
  const containersMb = containers.reduce((s, c) => s + c.mb, 0);
  const cats = [
    { id: "images", label: lang === "pt" ? "Imagens de container" : "Container images", mb: imagesMb, color: "#06b6d4", icon: "ti-stack-2" },
    { id: "containers", label: "Containers", mb: containersMb, color: "#06d6a0", icon: "ti-box" },
    ...STATIC,
  ];
  const used = cats.reduce((s, c) => s + c.mb, 0);
  const pct = (used / TOTAL) * 100;

  const unusedImages = images.filter((i) => i.usedBy === 0);
  const reclaimImg = unusedImages.reduce((s, i) => s + i.mb, 0);
  const stoppedCt = containers.filter((c) => c.state !== "running");
  const reclaimCt = stoppedCt.reduce((s, c) => s + c.mb, 0);

  const pruneImages = () => {
    if (!unusedImages.length) return;
    setImages((l) => l.filter((i) => i.usedBy > 0));
    push((lang === "pt" ? "Removidas " : "Removed ") + unusedImages.length + (lang === "pt" ? ` imagens · ${fmtGB(reclaimImg)} liberados` : ` images · ${fmtGB(reclaimImg)} freed`), "ti-trash");
  };
  const pruneContainers = () => {
    if (!stoppedCt.length) return;
    setContainers((l) => l.filter((c) => c.state === "running"));
    push((lang === "pt" ? "Removidos " : "Removed ") + stoppedCt.length + (lang === "pt" ? ` containers · ${fmtGB(reclaimCt)} liberados` : ` containers · ${fmtGB(reclaimCt)} freed`), "ti-trash");
  };

  const stLabel = (s) => s === "running" ? (lang === "pt" ? "em execução" : "running") : s === "stopped" ? (lang === "pt" ? "parado" : "stopped") : (lang === "pt" ? "saiu" : "exited");

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 20 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Disco" : "Disk"}</div>
          <div className="page-sub">{lang === "pt" ? "Uso de armazenamento do servidor" : "Server storage usage"}</div>
        </div>
        <button className="btn" onClick={() => push(lang === "pt" ? "Cache de sessões limpo" : "Session cache cleared", "ti-eraser")}><i className="ti ti-eraser"></i>{lang === "pt" ? "Limpar cache" : "Clear cache"}</button>
      </div>

      {loading ? (
        <div><A2.SkBlock w={"100%"} h={120} r={12} style={{ marginBottom: 14 }} /><A2.SkBlock w={"100%"} h={240} r={12} /></div>
      ) : (
        <React.Fragment>
          {/* usage hero */}
          <div className="disk-hero">
            <div className="disk-hero__head">
              <div>
                <div style={{ fontSize: 13, color: "var(--text-muted)", marginBottom: 4 }}>{lang === "pt" ? "Usado" : "Used"}</div>
                <div style={{ fontSize: 30, fontWeight: 600, letterSpacing: "-0.02em" }}><A2.LiveNum value={fmtGB(used)} /> <span style={{ fontSize: 15, color: "var(--text-faint)", fontWeight: 400 }}>/ {fmtGB(TOTAL)}</span></div>
              </div>
              <div style={{ textAlign: "right" }}>
                <div style={{ fontSize: 30, fontWeight: 600, letterSpacing: "-0.02em", color: pct > 80 ? "var(--warn)" : "var(--teal-600)" }}><A2.LiveNum value={pct.toFixed(1) + "%"} /></div>
                <div style={{ fontSize: 11.5, color: "var(--text-faint)" }}>{fmtGB(TOTAL - used)} {lang === "pt" ? "livres" : "free"}</div>
              </div>
            </div>
            <div className="disk-stack">
              {cats.map((c) => <div key={c.id} className="disk-stack__seg" title={c.label} style={{ width: (c.mb / TOTAL) * 100 + "%", background: c.color }}></div>)}
            </div>
            <div className="disk-legend">
              {cats.map((c) => (
                <div className="disk-legend__item" key={c.id}>
                  <span className="disk-legend__dot" style={{ background: c.color }}></span>
                  <i className={"ti " + c.icon} style={{ fontSize: 14, color: "var(--text-faint)" }}></i>
                  <span>{c.label}</span>
                  <span className="disk-legend__val tnum">{fmtGB(c.mb)}</span>
                </div>
              ))}
            </div>
          </div>

          {/* two-column: container images + containers */}
          <div className="disk-cols">
            {/* ── Container images ── */}
            <div className="disk-panel">
              <div className="disk-panel__head">
                <div>
                  <div className="disk-panel__title"><i className="ti ti-stack-2" style={{ color: "#06b6d4" }}></i>{lang === "pt" ? "Imagens de container" : "Container images"}</div>
                  <div className="disk-panel__sub">{images.length} {lang === "pt" ? "imagens" : "images"} · {fmtGB(imagesMb)}{unusedImages.length ? ` · ${unusedImages.length} ${lang === "pt" ? "não usadas" : "unused"}` : ""}</div>
                </div>
                <button className={"btn btn--danger" + (unusedImages.length ? "" : " is-disabled")} onClick={pruneImages}>
                  <i className="ti ti-trash"></i>{lang === "pt" ? "Limpar não usadas" : "Prune unused"}{unusedImages.length ? ` · ${fmtGB(reclaimImg)}` : ""}
                </button>
              </div>
              <table className="admin-table">
                <thead><tr>
                  <th>{lang === "pt" ? "Imagem" : "Image"}</th>
                  <th style={{ width: 80 }}>{lang === "pt" ? "Uso" : "In use"}</th>
                  <th style={{ width: 90, textAlign: "right" }}>{lang === "pt" ? "Tamanho" : "Size"}</th>
                  <th style={{ width: 44 }}></th>
                </tr></thead>
                <tbody>
                  {images.map((im) => (
                    <tr key={im.ref + im.tag} className={im.usedBy === 0 ? "is-unused" : ""}>
                      <td>
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <code className="mono" style={{ fontSize: 11.5 }}>{im.ref}<span className="faint">:{im.tag}</span></code>
                          {im.dangling && <span className="pill pill-unused">dangling</span>}
                        </div>
                      </td>
                      <td>{im.usedBy > 0
                        ? <span className="use-badge use-badge--on">{im.usedBy} {lang === "pt" ? "apps" : "apps"}</span>
                        : <span className="use-badge use-badge--off">{lang === "pt" ? "não usada" : "unused"}</span>}</td>
                      <td className="tnum" style={{ textAlign: "right", fontWeight: 500 }}>{fmtGB(im.mb)}</td>
                      <td style={{ textAlign: "right" }}><button className="star-toggle" title={lang === "pt" ? "Remover" : "Remove"} onClick={() => setImages((l) => l.filter((x) => x !== im))}><i className="ti ti-trash"></i></button></td>
                    </tr>
                  ))}
                  {images.length === 0 && <tr><td colSpan={4} style={{ textAlign: "center", color: "var(--text-faint)", padding: 24, fontSize: 12 }}>{lang === "pt" ? "Nenhuma imagem." : "No images."}</td></tr>}
                </tbody>
              </table>
            </div>

            {/* ── Containers ── */}
            <div className="disk-panel">
              <div className="disk-panel__head">
                <div>
                  <div className="disk-panel__title"><i className="ti ti-box" style={{ color: "#06d6a0" }}></i>Containers</div>
                  <div className="disk-panel__sub">{containers.length} {lang === "pt" ? "containers" : "containers"} · {fmtGB(containersMb)}{stoppedCt.length ? ` · ${stoppedCt.length} ${lang === "pt" ? "parados" : "stopped"}` : ""}</div>
                </div>
                <button className={"btn btn--danger" + (stoppedCt.length ? "" : " is-disabled")} onClick={pruneContainers}>
                  <i className="ti ti-trash"></i>{lang === "pt" ? "Limpar parados" : "Prune stopped"}{stoppedCt.length ? ` · ${fmtGB(reclaimCt)}` : ""}
                </button>
              </div>
              <table className="admin-table">
                <thead><tr>
                  <th>{lang === "pt" ? "Container" : "Container"}</th>
                  <th style={{ width: 120 }}>{lang === "pt" ? "Estado" : "State"}</th>
                  <th style={{ width: 90, textAlign: "right" }}>{lang === "pt" ? "Camada" : "Layer"}</th>
                  <th style={{ width: 44 }}></th>
                </tr></thead>
                <tbody>
                  {containers.map((ct) => (
                    <tr key={ct.cid} className={ct.state !== "running" ? "is-unused" : ""}>
                      <td><code className="mono" style={{ fontSize: 11.5 }}>{ct.cid}</code> <span className="faint" style={{ fontSize: 11 }}>· {ct.app}</span></td>
                      <td>
                        <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11.5 }}>
                          <span className={"dot " + (ct.state === "running" ? "dot-ready" : "dot-off")}></span>{stLabel(ct.state)}
                        </span>
                      </td>
                      <td className="tnum" style={{ textAlign: "right", fontWeight: 500 }}>{fmtGB(ct.mb)}</td>
                      <td style={{ textAlign: "right" }}><button className="star-toggle" title={lang === "pt" ? "Remover" : "Remove"} onClick={() => setContainers((l) => l.filter((x) => x !== ct))} disabled={ct.state === "running"} style={ct.state === "running" ? { opacity: 0.3, pointerEvents: "none" } : null}><i className="ti ti-trash"></i></button></td>
                    </tr>
                  ))}
                  {containers.length === 0 && <tr><td colSpan={4} style={{ textAlign: "center", color: "var(--text-faint)", padding: 24, fontSize: 12 }}>{lang === "pt" ? "Nenhum container." : "No containers."}</td></tr>}
                </tbody>
              </table>
            </div>
          </div>
        </React.Fragment>
      )}
    </div>
  );
}

/* ═══════════════════════════════════ CREDENTIALS ══════════════ */
const SCOPES = {
  pt: { admin: "Administrador", deploy: "Deploy", read: "Somente leitura" },
  en: { admin: "Administrator", deploy: "Deploy", read: "Read-only" },
};
function AdminCredentials({ t, loading, push }) {
  const lang = t._lang;
  const [creds, setCreds] = a2S(() => ([
    { id: 1, name: "CI / Deploy", scope: "deploy", token: "rk_live_a1c4", created: "12/03/2026", lastUsed: lang === "pt" ? "há 2 min" : "2 min ago", active: true },
    { id: 2, name: "Grafana scraper", scope: "read", token: "rk_live_9f2b", created: "28/02/2026", lastUsed: lang === "pt" ? "há 1 h" : "1 h ago", active: true },
    { id: 3, name: "Backup job", scope: "admin", token: "rk_live_77de", created: "04/01/2026", lastUsed: lang === "pt" ? "ontem" : "yesterday", active: true },
    { id: 4, name: "Laptop — Ana", scope: "read", token: "rk_live_3c0a", created: "15/12/2025", lastUsed: lang === "pt" ? "há 18 dias" : "18 days ago", active: false },
  ]));
  const [reveal, setReveal] = a2S(() => new Set());
  const toggleReveal = (id) => setReveal((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const revoke = (id) => { setCreds((c) => c.map((x) => x.id === id ? { ...x, active: false } : x)); push(lang === "pt" ? "Credencial revogada" : "Credential revoked", "ti-ban"); };

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 20 }}>
        <div>
          <div className="page-title">{lang === "pt" ? "Credenciais" : "Credentials"}</div>
          <div className="page-sub">{lang === "pt" ? "Tokens de API para deploy e integrações" : "API tokens for deploy and integrations"}</div>
        </div>
        <button className="btn btn--primary" onClick={() => push(lang === "pt" ? "Nova credencial — token gerado e copiado" : "New credential — token generated & copied", "ti-key")}><i className="ti ti-plus"></i>{lang === "pt" ? "Nova credencial" : "New credential"}</button>
      </div>

      {loading ? (
        <A2.SkBlock w={"100%"} h={240} r={12} />
      ) : (
        <table className="admin-table">
          <thead><tr>
            <th>{lang === "pt" ? "Nome" : "Name"}</th>
            <th style={{ width: 150 }}>{lang === "pt" ? "Escopo" : "Scope"}</th>
            <th style={{ width: 220 }}>Token</th>
            <th style={{ width: 120 }}>{lang === "pt" ? "Criado" : "Created"}</th>
            <th style={{ width: 120 }}>{lang === "pt" ? "Último uso" : "Last used"}</th>
            <th style={{ width: 110, textAlign: "right" }}>{lang === "pt" ? "Ações" : "Actions"}</th>
          </tr></thead>
          <tbody>
            {creds.map((c) => (
              <tr key={c.id} style={{ opacity: c.active ? 1 : 0.5 }}>
                <td><span style={{ display: "inline-flex", alignItems: "center", gap: 9, fontWeight: 500 }}>
                  <span className="cred-ico"><i className="ti ti-key"></i></span>{c.name}
                  {!c.active && <span className="pill" style={{ background: "var(--surface-soft)", color: "var(--text-faint)" }}>{lang === "pt" ? "REVOGADA" : "REVOKED"}</span>}
                </span></td>
                <td><span className={"pill pill-scope-" + c.scope}>{SCOPES[lang][c.scope]}</span></td>
                <td>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
                    <code className="mono" style={{ fontSize: 12 }}>{reveal.has(c.id) ? c.token + "8e1d2f" : c.token.slice(0, 7) + "••••••"}</code>
                    <button className="star-toggle" style={{ width: 26, height: 26 }} title={reveal.has(c.id) ? (lang === "pt" ? "Ocultar" : "Hide") : (lang === "pt" ? "Mostrar" : "Show")} onClick={() => toggleReveal(c.id)}><i className={"ti " + (reveal.has(c.id) ? "ti-eye-off" : "ti-eye")} style={{ fontSize: 14 }}></i></button>
                    <button className="star-toggle" style={{ width: 26, height: 26 }} title={lang === "pt" ? "Copiar" : "Copy"} onClick={() => push(lang === "pt" ? "Token copiado" : "Token copied", "ti-copy")}><i className="ti ti-copy" style={{ fontSize: 14 }}></i></button>
                  </span>
                </td>
                <td className="muted tnum" style={{ fontSize: 11.5 }}>{c.created}</td>
                <td className="muted" style={{ fontSize: 11.5 }}>{c.lastUsed}</td>
                <td style={{ textAlign: "right" }}>
                  {c.active && <button className="star-toggle" title={lang === "pt" ? "Revogar" : "Revoke"} onClick={() => revoke(c.id)}><i className="ti ti-ban"></i></button>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/* ══════════════════════════════════════ LOGS ══════════════════ */
const LOG_TPL = [
  { lvl: "info", msg: (a) => `request served · 200 · ${(8 + Math.random() * 40).toFixed(0)}ms`, app: true },
  { lvl: "info", msg: (a) => `session started · ${Math.random().toString(16).slice(2, 8)}`, app: true },
  { lvl: "info", msg: () => `load balancer · routing to replica node-${["a", "b"][Math.floor(Math.random() * 2)]}`, app: true },
  { lvl: "warn", msg: (a) => `replica memory at ${(70 + Math.random() * 20).toFixed(0)}% · ${a}`, app: true },
  { lvl: "info", msg: () => `health check ok`, app: true },
  { lvl: "error", msg: (a) => `upstream timeout after 30s · ${a} · retrying`, app: true },
  { lvl: "warn", msg: () => `session pool near capacity (9/10)`, app: true },
  { lvl: "info", msg: (a) => `container ready · ${a}`, app: true },
];
function AdminLogs({ t, loading, push }) {
  const lang = t._lang;
  const APPS = window.RK.APPS;
  const [lines, setLines] = a2S([]);
  const [paused, setPaused] = a2S(false);
  const [lvl, setLvl] = a2S("all");
  const [appF, setAppF] = a2S("all");
  const bodyRef = a2R(null);
  const seq = a2R(0);

  const mkLine = () => {
    const tpl = LOG_TPL[Math.floor(Math.random() * LOG_TPL.length)];
    const app = APPS[Math.floor(Math.random() * 8)];
    const now = new Date();
    const ts = now.toTimeString().slice(0, 8) + "." + String(now.getMilliseconds()).padStart(3, "0");
    return { id: ++seq.current, ts, lvl: tpl.lvl, app: app.name, msg: tpl.msg(app.name) };
  };

  // seed + live stream
  a2E(() => {
    if (loading) return undefined;
    if (lines.length === 0) setLines(Array.from({ length: 14 }, mkLine));
    if (paused) return undefined;
    const iv = setInterval(() => setLines((l) => [...l.slice(-140), mkLine()]), 1100);
    return () => clearInterval(iv);
  }, [loading, paused]);

  // autoscroll
  a2E(() => { if (!paused && bodyRef.current) bodyRef.current.scrollTop = bodyRef.current.scrollHeight; }, [lines, paused]);

  const shown = lines.filter((l) => (lvl === "all" || l.lvl === lvl) && (appF === "all" || l.app === appF));
  const LvlChip = ({ k, label }) => <button className={"chip" + (lvl === k ? " on-all" : "")} onClick={() => setLvl(lvl === k ? "all" : k)}>{label}</button>;

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 16 }}>
        <div>
          <div className="page-title">Logs</div>
          <div className="page-sub">{lang === "pt" ? "Stream de eventos do balanceador e das réplicas" : "Event stream from the balancer and replicas"}</div>
        </div>
        <span style={{ display: "inline-flex", alignItems: "center", gap: 7, fontSize: 11.5, color: "var(--text-muted)" }}>
          {!paused && <span className="live-dot"></span>}{paused ? (lang === "pt" ? "pausado" : "paused") : t.live}
        </span>
      </div>

      <div className="ops-toolbar" style={{ marginBottom: 0, borderRadius: "12px 12px 0 0", border: "0.5px solid var(--border)", borderBottom: 0, background: "var(--surface-soft)", padding: "10px 12px" }}>
        <button className="sort-btn" onClick={() => setPaused((p) => !p)}><i className={"ti " + (paused ? "ti-player-play" : "ti-player-pause")}></i>{paused ? (lang === "pt" ? "Retomar" : "Resume") : (lang === "pt" ? "Pausar" : "Pause")}</button>
        <span className="chip-divider"></span>
        <LvlChip k="info" label="Info" />
        <LvlChip k="warn" label={lang === "pt" ? "Alerta" : "Warn"} />
        <LvlChip k="error" label={lang === "pt" ? "Erro" : "Error"} />
        <select className="log-select" value={appF} onChange={(e) => setAppF(e.target.value)}>
          <option value="all">{lang === "pt" ? "Todos os apps" : "All apps"}</option>
          {APPS.slice(0, 8).map((a) => <option key={a.id} value={a.name}>{a.name}</option>)}
        </select>
        <div style={{ flex: 1 }}></div>
        <span className="faint tnum" style={{ fontSize: 11 }}>{shown.length} {lang === "pt" ? "linhas" : "lines"}</span>
        <button className="star-toggle" title={lang === "pt" ? "Limpar" : "Clear"} onClick={() => setLines([])}><i className="ti ti-eraser"></i></button>
        <button className="star-toggle" title={lang === "pt" ? "Baixar" : "Download"} onClick={() => push(lang === "pt" ? "Baixando logs…" : "Downloading logs…", "ti-download")}><i className="ti ti-download"></i></button>
      </div>

      {loading ? (
        <div className="log-body" style={{ padding: 14 }}>{Array.from({ length: 10 }).map((_, i) => <A2.SkBlock key={i} w={(50 + Math.random() * 45) + "%"} h={9} style={{ marginBottom: 11 }} />)}</div>
      ) : (
        <div className="log-body" ref={bodyRef}>
          {shown.map((l) => (
            <div className="log-line" key={l.id}>
              <span className="log-ts">{l.ts}</span>
              <span className={"log-lvl log-lvl-" + l.lvl}>{l.lvl.toUpperCase()}</span>
              <span className="log-app">{l.app}</span>
              <span className="log-msg">{l.msg}</span>
            </div>
          ))}
          {shown.length === 0 && <div style={{ color: "var(--text-faint)", fontSize: 12, padding: 20, textAlign: "center" }}>{lang === "pt" ? "Sem linhas para este filtro." : "No lines for this filter."}</div>}
        </div>
      )}
    </div>
  );
}

window.RKAdmin2 = { AdminDisk, AdminCredentials, AdminLogs };
