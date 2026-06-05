/* global React, window */
// ── Dashboard (monitoring) — 3 variations ──
const { useState: dS, useMemo: dM, useEffect: dE, useRef: dR } = React;
const DC = window.RKC;

const clampN = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const randN = (a, b) => a + Math.random() * (b - a);

function aggregate(g) {
  const reps = g.reps;
  const sessions = reps.reduce((s, r) => s + r.sessions, 0);
  const sessionsMax = reps.reduce((s, r) => s + r.sessionsMax, 0);
  const mem = reps.reduce((s, r) => s + r.mem, 0);
  const cpu = Math.round(reps.reduce((s, r) => s + r.cpu, 0) / reps.length);
  const boot = reps.some((r) => r.state === "boot");
  const warn = reps.some((r) => r.state === "warn");
  const state = boot ? "boot" : warn ? "warn" : "ready";
  return { sessions, sessionsMax, mem, cpu, state, n: reps.length };
}

function StateDot({ s }) {
  const cls = s === "boot" ? "dot-boot" : s === "warn" ? "dot-warn" : s === "stopped" ? "dot-off" : "dot-ready";
  return <span className={"dot " + cls}></span>;
}
function stateLabel(s, t) { return s === "boot" ? t.booting : s === "warn" ? t.warn : s === "stopped" ? t.stopped : t.ready; }

function Dashboard({ variation, t, loading, push }) {
  const init = dM(() => window.RK.buildReplicas(), []);
  const [groups, setGroups] = dS(init.groups);
  const total = init.total;

  // ── Live data: random-walk metrics every 2.5s; the booting replica
  //    progresses to ready (visible real progress). Sells responsiveness.
  dE(() => {
    if (loading) return undefined;
    const iv = setInterval(() => {
      setGroups((gs) => gs.map((g) => ({
        ...g,
        reps: g.reps.map((r) => {
          if (r.state === "boot") {
            const prog = (r.bootProg || 0) + 1;
            if (prog >= 3) return { ...r, state: "ready", bootProg: prog, uptime: "0h 1m", cpu: Math.round(randN(4, 10)), mem: 140 + Math.round(randN(0, 70)), sessions: 0 };
            return { ...r, bootProg: prog };
          }
          const cap = r.state === "warn" ? 42 : 30;
          const cpu = Math.round(clampN(r.cpu + randN(-5, 5), 1, cap));
          const ds = Math.random() < 0.28 ? (Math.random() < 0.5 ? -1 : 1) : 0;
          const sessions = clampN(r.sessions + ds, 0, r.sessionsMax);
          const mem = Math.round(clampN(r.mem + randN(-14, 14), 80, 900));
          return { ...r, cpu, sessions, mem };
        }),
      })));
    }, 2500);
    return () => clearInterval(iv);
  }, [loading]);

  const totals = (() => {
    const sessions = groups.reduce((s, g) => s + aggregate(g).sessions, 0);
    const mem = groups.reduce((s, g) => s + aggregate(g).mem, 0);
    return { containers: total, appsWith: groups.length, sessions, tracked: sessions, mem };
  })();

  if (variation === "b") return <DashCards t={t} groups={groups} totals={totals} loading={loading} push={push} />;
  if (variation === "c") return <DashOps t={t} groups={groups} totals={totals} loading={loading} push={push} />;
  return <DashGrouped t={t} groups={groups} totals={totals} loading={loading} push={push} />;
}

/* ── metrics strip (shared) ────────────────────────────────────── */
function Metrics({ t, totals, loading }) {
  const memStr = totals.mem > 1024 ? (totals.mem / 1024).toFixed(1) + " GB" : totals.mem + " MB";
  const items = [
    { icon: "ti-box", label: t.containers, value: totals.containers, sub: t.lastRefresh },
    { icon: "ti-stack-2", label: t.appsReplicas, value: totals.appsWith },
    { icon: "ti-users", label: t.activeSessions, value: totals.sessions, sub: t.peakToday + ": 134" },
    { icon: "ti-activity", label: t.tracked, value: totals.tracked },
    { icon: "ti-database", label: t.memUsed, value: memStr },
  ];
  return (
    <div className="dash-metrics">
      {items.map((m, i) => (
        <div className="metric" key={i}>
          <div className="metric__label"><i className={"ti " + m.icon}></i>{m.label}</div>
          {loading ? <DC.SkBlock w={70} h={26} /> : <div className="metric__value"><DC.LiveNum value={m.value} /></div>}
          {m.sub && !loading && <div className="metric__sub">{m.sub}</div>}
        </div>
      ))}
    </div>
  );
}

function DashHeader({ t, right }) {
  return (
    <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 16, marginBottom: 20 }}>
      <div>
        <div className="page-title">{t.dashboard === "Painel" ? "Painel de monitoramento" : "Monitoring dashboard"}</div>
        <div className="page-sub">{t.dashSub}</div>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>{right}</div>
    </div>
  );
}

function LiveBadge({ t }) {
  return <span style={{ display: "inline-flex", alignItems: "center", gap: 7, fontSize: 11.5, color: "var(--text-muted)" }}><span className="live-dot"></span>{t.live} · 2.5s</span>;
}

/* ══════════════════════════════════════════════════════════════
   VARIATION A — Grouped by app (collapse / expand)
   ══════════════════════════════════════════════════════════════ */
function DashGrouped({ t, groups, totals, loading, push }) {
  const [open, setOpen] = dS(() => new Set());
  const [q, setQ] = dS("");
  const [stateF, setStateF] = dS("all");
  const allOpen = open.size === groups.length;
  const toggle = (id) => setOpen((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });

  const shown = groups.filter((g) => {
    if (q.trim() && !(g.name + " " + g.id).toLowerCase().includes(q.trim().toLowerCase())) return false;
    if (stateF !== "all" && aggregate(g).state !== stateF) return false;
    return true;
  });
  const toggleAll = () => setOpen(allOpen ? new Set() : new Set(shown.map((g) => g.id)));

  const StateChip = ({ k, label }) => (
    <button className={"chip" + (stateF === k ? " on-all" : "")} onClick={() => setStateF(stateF === k ? "all" : k)}>
      {k !== "all" && <StateDot s={k} />}{label}
    </button>
  );

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <DashHeader t={t} right={<LiveBadge t={t} />} />
      <Metrics t={t} totals={totals} loading={loading} />

      {/* Sticky control + column-header band: stays pinned while the long
          grouped list scrolls — filters & headers always reachable. */}
      <div className="grouped-sticky">
        <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap", marginBottom: 10 }}>
          <div style={{ fontSize: 14, fontWeight: 600, display: "flex", alignItems: "center", gap: 9, marginRight: 4 }}>
            {t.activeReplicas}
            <span style={{ fontSize: 11, fontWeight: 400, color: "var(--text-faint)" }}>· {t.groupedByApp}</span>
          </div>
          <div className="search-pill" style={{ maxWidth: 230, flex: "0 1 230px", padding: "8px 14px" }}>
            <i className="ti ti-search"></i>
            <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={t.filterApp} />
          </div>
          <StateChip k="ready" label={t.ready} />
          <StateChip k="warn" label={t.warn} />
          <StateChip k="boot" label={t.booting} />
          <div style={{ flex: 1 }}></div>
          <span className="faint tnum" style={{ fontSize: 11 }}>{shown.length}/{groups.length}</span>
          {!loading && (
            <button className="sort-btn" onClick={toggleAll}>
              <i className={"ti " + (allOpen ? "ti-fold" : "ti-fold-down")}></i>{allOpen ? t.collapseAll : t.expandAll}
            </button>
          )}
        </div>
        {/* column header */}
        <div className="app-group__head" style={{ padding: "6px 16px 8px", cursor: "default", background: "transparent" }}>
          <span></span>
          <span className="col-h">App</span>
          <span className="col-h">{t.replicas}</span>
          <span className="col-h">{t.state}</span>
          <span className="col-h">{t.sessions}</span>
          <span className="col-h">{t.cpu}</span>
          <span className="col-h">{t.mem}</span>
          <span></span>
        </div>
      </div>

      {loading
        ? Array.from({ length: 6 }).map((_, i) => (
            <div className="app-group" key={i} style={{ padding: "14px 16px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <DC.SkBlock w={16} h={16} /><DC.SkBlock w={30} h={30} r={8} />
                <DC.SkBlock w={120} h={12} /><div style={{ flex: 1 }}></div><DC.SkBlock w={200} h={10} />
              </div>
            </div>
          ))
        : shown.length === 0
        ? <div style={{ textAlign: "center", padding: "48px 20px", color: "var(--text-muted)" }}><i className="ti ti-server-off" style={{ fontSize: 30, color: "var(--text-faint)" }}></i><div style={{ marginTop: 10, fontSize: 13 }}>{t.noResults}</div></div>
        : <div className="stagger" style={{ marginTop: 12 }}>{shown.map((g, i) => {
            const a = aggregate(g);
            const isOpen = open.has(g.id);
            return (
              <div className={"app-group" + (isOpen ? " is-open" : "")} key={g.id} style={{ animationDelay: Math.min(i, 10) * 10 + "ms" }}>
                <div className="app-group__head" onClick={() => toggle(g.id)}>
                  <span className="app-group__chev"><i className="ti ti-chevron-right"></i></span>
                  <span className="app-group__name">
                    <DC.Logo app={g} size={30} radius={8} />
                    <span>{g.name}<span className="app-group__id" style={{ display: "block" }}>{g.id}</span></span>
                  </span>
                  <span className="app-group__replicas"><i className="ti ti-stack-2" style={{ fontSize: 14 }}></i>{a.n}</span>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 7, fontSize: 12 }}><StateDot s={a.state} />{stateLabel(a.state, t)}</span>
                  <span className="metric-cell"><span className="v tnum">{a.sessions}<span className="faint">/{a.sessionsMax}</span></span><DC.Meter pct={(a.sessions / a.sessionsMax) * 100} color="var(--info)" /></span>
                  <span className="metric-cell"><span className="v tnum">{a.cpu}%</span><DC.Meter pct={a.cpu} color={DC.meterColor(a.cpu)} /></span>
                  <span className="metric-cell tnum">{a.mem} MB</span>
                  <span style={{ textAlign: "right", color: "var(--text-faint)" }}><i className="ti ti-dots-vertical"></i></span>
                </div>
                {isOpen && (
                  <div className="replica-list fade-in">
                    {g.reps.map((r) => (
                      <div className="replica-row" key={r.cid}>
                        <span><StateDot s={r.state} /></span>
                        <span><span className="cid">{r.cid}</span> <span className="faint" style={{ fontSize: 11 }}>· {r.host}</span></span>
                        <span className="mono faint" style={{ fontSize: 11 }}>{r.uptime}</span>
                        <span style={{ fontSize: 11.5 }} className="muted">{stateLabel(r.state, t)}</span>
                        <span className="metric-cell"><span className="tnum">{r.sessions}<span className="faint">/{r.sessionsMax}</span></span></span>
                        <span className="metric-cell"><span className="tnum">{r.cpu}%</span><DC.Meter pct={r.cpu} color={DC.meterColor(r.cpu)} /></span>
                        <span className="metric-cell tnum">{r.mem} MB</span>
                        <span className="replica-actions">
                          <button title={t.logs} onClick={() => push(t.logs + " · " + r.cid.slice(0, 6), "ti-file-text")}><i className="ti ti-file-text"></i></button>
                          <button title={t.restart} onClick={() => push(t.restart + " · " + r.cid.slice(0, 6), "ti-refresh")}><i className="ti ti-refresh"></i></button>
                          <button title={t.stop} onClick={() => push(t.stop + " · " + r.cid.slice(0, 6), "ti-square")}><i className="ti ti-square"></i></button>
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}</div>}
    </div>
  );
}

/* ══════════════════════════════════════════════════════════════
   VARIATION B — App cards + drill-in detail (wayfinding)
   ══════════════════════════════════════════════════════════════ */
function DashCards({ t, groups, totals, loading, push }) {
  const [detail, setDetail] = dS(null);
  const sel = detail ? groups.find((g) => g.id === detail) : null;

  if (sel) {
    const a = aggregate(sel);
    return (
      <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
        <div className="crumbs">
          <a onClick={() => setDetail(null)}>{t.dashboard}</a><i className="ti ti-chevron-right"></i>
          <span style={{ color: "var(--text)" }}>{sel.name}</span>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 13, marginBottom: 20 }}>
          <DC.Logo app={sel} size={44} radius={11} />
          <div style={{ flex: 1 }}>
            <div className="page-title" style={{ fontSize: 22 }}>{sel.name}</div>
            <div className="page-sub mono">{sel.id} · {a.n} {t.replicas}</div>
          </div>
          <button className="btn" onClick={() => setDetail(null)}><i className="ti ti-arrow-left"></i>{t.backToList}</button>
        </div>
        <div className="dash-metrics" style={{ gridTemplateColumns: "repeat(3,1fr)", maxWidth: 560 }}>
          <div className="metric"><div className="metric__label"><i className="ti ti-users"></i>{t.totalSessions}</div><div className="metric__value tnum">{a.sessions}<small>/{a.sessionsMax}</small></div></div>
          <div className="metric"><div className="metric__label"><i className="ti ti-cpu"></i>{t.avgCpu}</div><div className="metric__value tnum">{a.cpu}<small>%</small></div></div>
          <div className="metric"><div className="metric__label"><i className="ti ti-database"></i>{t.totalMem}</div><div className="metric__value tnum">{a.mem}<small> MB</small></div></div>
        </div>
        <div style={{ height: 18 }}></div>
        <div className="app-group is-open">
          {sel.reps.map((r) => (
            <div className="replica-row" key={r.cid} style={{ gridTemplateColumns: "24px 1.6fr 80px 90px 1fr 1fr 100px 40px" }}>
              <span><StateDot s={r.state} /></span>
              <span><span className="cid">{r.cid}</span> <span className="faint">· {r.host}</span></span>
              <span className="mono faint" style={{ fontSize: 11 }}>{r.uptime}</span>
              <span className="muted" style={{ fontSize: 11.5 }}>{stateLabel(r.state, t)}</span>
              <span className="tnum">{r.sessions}<span className="faint">/{r.sessionsMax}</span></span>
              <span className="metric-cell"><span className="tnum">{r.cpu}%</span><DC.Meter pct={r.cpu} color={DC.meterColor(r.cpu)} /></span>
              <span className="tnum">{r.mem} MB</span>
              <span className="replica-actions">
                <button onClick={() => push(t.restart + " · " + r.cid.slice(0, 6), "ti-refresh")}><i className="ti ti-refresh"></i></button>
              </span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <DashHeader t={t} right={<LiveBadge t={t} />} />
      <Metrics t={t} totals={totals} loading={loading} />
      <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 12 }}>{t.activeReplicas}</div>
      {loading ? (
        <div className="dash-cards">{Array.from({ length: 6 }).map((_, i) => (
          <div className="dcard" key={i} style={{ cursor: "default" }}>
            <div style={{ display: "flex", gap: 11, marginBottom: 14 }}><DC.SkBlock w={38} h={38} r={9} /><div style={{ flex: 1 }}><DC.SkBlock w={"50%"} h={12} style={{ marginBottom: 7 }} /><DC.SkBlock w={"30%"} h={9} /></div></div>
            <DC.SkBlock w={"100%"} h={40} /><DC.SkBlock w={"100%"} h={30} style={{ marginTop: 14 }} />
          </div>
        ))}</div>
      ) : (
        <div className="dash-cards stagger">
          {groups.map((g, i) => {
            const a = aggregate(g);
            const hp = a.state === "boot" ? "boot" : a.state === "warn" ? "warn" : "ok";
            const spark = DC.makeSpark(24, 40 + i * 3, 26, i + 1);
            return (
              <div className="dcard" key={g.id} style={{ animationDelay: Math.min(i, 10) * 11 + "ms" }} onClick={() => setDetail(g.id)}>
                <div className="dcard__top">
                  <DC.Logo app={g} size={38} radius={9} />
                  <div style={{ minWidth: 0 }}><div className="dcard__name">{g.name}</div><div className="dcard__id">{g.id}</div></div>
                  <span className={"health-pill " + hp}>
                    <StateDot s={a.state} />{a.state === "ok" || a.state === "ready" ? t.healthy : stateLabel(a.state, t)}
                  </span>
                </div>
                <div className="dcard__stats">
                  <div className="dcard__stat"><div className="k"><i className="ti ti-stack-2"></i> {t.replicas}</div><div className="v tnum">{a.n}</div></div>
                  <div className="dcard__stat"><div className="k">{t.sessions}</div><div className="v tnum">{a.sessions}</div></div>
                  <div className="dcard__stat"><div className="k">{t.cpu}</div><div className="v tnum">{a.cpu}%</div></div>
                </div>
                <div className="dcard__spark"><DC.Sparkline data={spark} color={g.accent} /></div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/* ══════════════════════════════════════════════════════════════
   VARIATION C — Dense ops table (grouped, sticky, filterable)
   ══════════════════════════════════════════════════════════════ */
function DashOps({ t, groups, totals, loading, push }) {
  const [open, setOpen] = dS(() => new Set(groups.slice(0, 2).map((g) => g.id)));
  const [filter, setFilter] = dS("");
  const [stateF, setStateF] = dS("all");
  const toggle = (id) => setOpen((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });

  const shown = groups.filter((g) => {
    if (filter.trim() && !(g.name + " " + g.id).toLowerCase().includes(filter.trim().toLowerCase())) return false;
    if (stateF !== "all" && aggregate(g).state !== stateF) return false;
    return true;
  });

  const StateChip = ({ k, label }) => (
    <button className={"chip" + (stateF === k ? " on-all" : "")} onClick={() => setStateF(stateF === k ? "all" : k)}>{label}</button>
  );

  return (
    <div className="mock-pad" style={{ paddingTop: 16, paddingBottom: 40 }}>
      <DashHeader t={t} right={<LiveBadge t={t} />} />
      <Metrics t={t} totals={totals} loading={loading} />

      <div className="ops-toolbar">
        <div className="search-pill" style={{ maxWidth: 300, flex: "0 1 300px" }}>
          <i className="ti ti-search"></i>
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t.filterApp} />
        </div>
        <span className="chip-divider"></span>
        <StateChip k="ready" label={t.ready} />
        <StateChip k="warn" label={t.warn} />
        <StateChip k="boot" label={t.booting} />
        <div style={{ flex: 1 }}></div>
        <span className="faint" style={{ fontSize: 11 }}>{t.lastRefresh}</span>
      </div>

      {loading ? (
        <div className="ops-table" style={{ padding: 14 }}>{Array.from({ length: 8 }).map((_, i) => <DC.SkBlock key={i} w={"100%"} h={14} style={{ marginBottom: 12 }} />)}</div>
      ) : (
        <table className="ops-table">
          <thead>
            <tr>
              <th style={{ width: 26 }}></th>
              <th>App / {t.container}</th>
              <th style={{ width: 70 }}>{t.replicas}</th>
              <th style={{ width: 90 }}>{t.state}</th>
              <th style={{ width: 80 }}>{t.uptime}</th>
              <th style={{ width: 70 }}>{t.sessions}</th>
              <th style={{ width: 90 }}>{t.cpu}</th>
              <th style={{ width: 80 }}>{t.mem}</th>
              <th style={{ width: 110 }}>24h</th>
              <th style={{ width: 40 }}></th>
            </tr>
          </thead>
          <tbody>
            {shown.map((g, gi) => {
              const a = aggregate(g);
              const isOpen = open.has(g.id);
              const spark = DC.makeSpark(20, 40 + gi * 2, 22, gi + 2);
              return (
                <React.Fragment key={g.id}>
                  <tr className="grp-row" onClick={() => toggle(g.id)}>
                    <td><i className="ti ti-chevron-right" style={{ transition: "transform .2s", transform: isOpen ? "rotate(90deg)" : "none", color: "var(--text-faint)" }}></i></td>
                    <td><span style={{ display: "inline-flex", alignItems: "center", gap: 9 }}><DC.Logo app={g} size={24} radius={6} />{g.name} <span className="mono faint" style={{ fontSize: 10.5 }}>{g.id}</span></span></td>
                    <td className="tnum">{a.n}</td>
                    <td><span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}><StateDot s={a.state} />{stateLabel(a.state, t)}</span></td>
                    <td className="faint">—</td>
                    <td className="tnum">{a.sessions}/{a.sessionsMax}</td>
                    <td className="tnum">{a.cpu}%</td>
                    <td className="tnum">{a.mem} MB</td>
                    <td><DC.Sparkline data={spark} color={g.accent} h={22} /></td>
                    <td></td>
                  </tr>
                  {isOpen && g.reps.map((r) => (
                    <tr key={r.cid} className="fade-in">
                      <td></td>
                      <td style={{ paddingLeft: 46 }}><span className="cid">{r.cid}</span> <span className="faint">· {r.host}</span></td>
                      <td></td>
                      <td><span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}><StateDot s={r.state} /><span className="muted">{stateLabel(r.state, t)}</span></span></td>
                      <td className="mono faint">{r.uptime}</td>
                      <td className="tnum">{r.sessions}/{r.sessionsMax}</td>
                      <td className="tnum">{r.cpu}%</td>
                      <td className="tnum">{r.mem} MB</td>
                      <td></td>
                      <td className="replica-actions">
                        <button onClick={(e) => { e.stopPropagation(); push(t.restart + " · " + r.cid.slice(0, 6), "ti-refresh"); }}><i className="ti ti-refresh"></i></button>
                      </td>
                    </tr>
                  ))}
                </React.Fragment>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}

window.RKDashboard = Dashboard;
