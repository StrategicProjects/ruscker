/* global React */
// ── Shared visual atoms + skeletons for the Ruscker prototype ──
const { useState, useEffect, useRef } = React;

// Monogram logo on a soft accent plate — renders the real brand mark when
// available (app.logo), falling back to the monogram if it errors/missing.
function Logo({ app, size = 40, radius = 9 }) {
  const [err, setErr] = useState(false);
  const bg = `color-mix(in srgb, ${app.accent} 16%, var(--surface))`;
  const base = {
    width: size, height: size, borderRadius: radius, flex: "none",
    display: "flex", alignItems: "center", justifyContent: "center",
    background: bg, border: "0.5px solid var(--border)", overflow: "hidden",
  };
  if (app.logo && !err) {
    return (
      <div style={base}>
        <img src={app.logo} alt="" onError={() => setErr(true)}
             style={{ width: size * 0.62, height: size * 0.62, objectFit: "contain", display: "block" }} />
      </div>
    );
  }
  return <div style={{ ...base, color: app.accent, fontWeight: 600, fontSize: size * 0.4 }}>{app.mono}</div>;
}

// Big cover logo for the card — real brand mark on a tinted radial, with the
// name wordmark as fallback when no logo is set / it fails to load.
function CoverLogo({ app, coverStyle }) {
  const k = window.RK.KIND[app.kind];
  const [err, setErr] = useState(false);
  const showImg = app.logo && !err;
  return (
    <div className="rcover">
      <div className={"rcover-bg " + (coverStyle === "gradient" ? "" : k.tint)} style={coverStyle === "gradient" ? { background: `linear-gradient(140deg, color-mix(in srgb, ${app.accent} 30%, transparent), color-mix(in srgb, ${app.accent} 6%, transparent))` } : null}></div>
      <div className="rcover-logo" style={{ color: app.accent }}>
        {showImg ? (
          <img src={app.logo} alt={app.name} onError={() => setErr(true)}
               style={{ maxWidth: "54%", maxHeight: "56%", objectFit: "contain", display: "block" }} />
        ) : (
          <span style={{
            fontSize: 26, fontWeight: 600, letterSpacing: "-0.02em",
            fontFamily: app.kind === "api" ? "var(--font-mono)" : "inherit",
          }}>{app.name}</span>
        )}
      </div>
      <div className="rbadges">
        <span className={"rbadge " + k.b}>{k.badge}</span>
        <span className="rbadge rbadge--subject">{app.subject}</span>
      </div>
      <span className="rlock">
        <i className={"ti " + (app.locked ? "ti-lock" : "ti-lock-open")}
           style={{ color: app.locked ? "var(--lock)" : "var(--lock-open)" }}></i>
      </span>
    </div>
  );
}

function StatusMeta({ app, t }) {
  if (app.status === "new") return <span className="rmeta-left"><span className="status-dot status-dot-new"></span>{t.newBadge} {app.updated}</span>;
  if (app.status === "updated") return <span className="rmeta-left"><span className="status-dot status-dot-updated"></span>{t.updatedBadge} {app.updated}</span>;
  return <span className="rmeta-left"><span className="status-dot" style={{ background: "var(--text-faint)" }}></span>{t.updated} {app.updated}</span>;
}

// Full app card
function AppCard({ app, t, onOpen, coverStyle }) {
  return (
    <a className="rcard" onClick={(e) => { e.preventDefault(); onOpen && onOpen(app); }}>
      <CoverLogo app={app} coverStyle={coverStyle} />
      <div className="rbody">
        <div className="rtitle">{app.name}</div>
        <div className="rdesc">{app.desc[t._lang]}</div>
        <div className="rmeta">
          <StatusMeta app={app} t={t} />
          <i className="ti ti-arrow-right arrow-go"></i>
        </div>
      </div>
    </a>
  );
}

// Compact list row card
function AppRow({ app, t, onOpen }) {
  const k = window.RK.KIND[app.kind];
  return (
    <a className="rrow" onClick={(e) => { e.preventDefault(); onOpen && onOpen(app); }}>
      <Logo app={app} size={42} />
      <div className="rrow__main">
        <div className="rrow__title">
          {app.name}
          <span className={"rbadge " + k.b} style={{ fontSize: 8 }}>{k.badge}</span>
        </div>
        <div className="rrow__desc">{app.desc[t._lang]}</div>
      </div>
      <div className="rrow__meta">
        <i className={"ti " + (app.locked ? "ti-lock" : "ti-lock-open")}
           style={{ color: app.locked ? "var(--lock)" : "var(--lock-open)", fontSize: 13 }}></i>
        <span>{t.updated} {app.updated}</span>
        <i className="ti ti-arrow-right arrow-go"></i>
      </div>
    </a>
  );
}

/* ── Skeletons ────────────────────────────────────────────────── */
function SkCard() {
  return (
    <div className="rcard" style={{ cursor: "default" }}>
      <div className="sk" style={{ height: 150, margin: "10px 10px 0", borderRadius: 8 }}></div>
      <div className="rbody">
        <div className="sk sk-line" style={{ width: "60%", marginBottom: 10 }}></div>
        <div className="sk sk-line" style={{ width: "92%", marginBottom: 6, height: 8 }}></div>
        <div className="sk sk-line" style={{ width: "78%", marginBottom: 16, height: 8 }}></div>
        <div className="sk sk-line" style={{ width: "40%", height: 8 }}></div>
      </div>
    </div>
  );
}
function SkRow() {
  return (
    <div className="rrow" style={{ cursor: "default" }}>
      <div className="sk" style={{ width: 42, height: 42, borderRadius: 9, flex: "none" }}></div>
      <div style={{ flex: 1 }}>
        <div className="sk sk-line" style={{ width: "35%", marginBottom: 8 }}></div>
        <div className="sk sk-line" style={{ width: "70%", height: 8 }}></div>
      </div>
      <div className="sk sk-line" style={{ width: 70, height: 8 }}></div>
    </div>
  );
}
function SkBlock({ h = 12, w = "100%", r = 6, style }) {
  return <div className="sk" style={{ height: h, width: w, borderRadius: r, ...style }}></div>;
}

/* ── Sparkline ────────────────────────────────────────────────── */
function makeSpark(n = 24, base = 40, jitter = 30, seed = 1) {
  const arr = [];
  let v = base;
  for (let i = 0; i < n; i++) {
    v += (Math.sin(i * 0.6 + seed) * jitter * 0.4) + (Math.random() - 0.5) * jitter;
    v = Math.max(4, Math.min(96, v));
    arr.push(v);
  }
  return arr;
}
function Sparkline({ data, color = "var(--teal-600)", fill = true, h = 30 }) {
  const w = 120;
  const max = Math.max(...data), min = Math.min(...data);
  const span = Math.max(1, max - min);
  const pts = data.map((d, i) => {
    const x = (i / (data.length - 1)) * w;
    const y = h - 3 - ((d - min) / span) * (h - 6);
    return [x, y];
  });
  const line = pts.map((p, i) => (i === 0 ? "M" : "L") + p[0].toFixed(1) + " " + p[1].toFixed(1)).join(" ");
  const area = line + ` L ${w} ${h} L 0 ${h} Z`;
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" style={{ height: h }}>
      {fill && <path d={area} fill={color} opacity="0.12" />}
      <path d={line} fill="none" stroke={color} strokeWidth="1.6" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/* ── Bar (cpu/mem mini meter) ─────────────────────────────────── */
function Meter({ pct, color }) {
  return <div className="bar-track" style={{ marginTop: 4 }}><div className="bar-fill" style={{ width: pct + "%", background: color }}></div></div>;
}
function meterColor(pct) {
  if (pct >= 80) return "var(--warn)";
  if (pct >= 50) return "var(--info)";
  return "var(--ok)";
}

/* ── Toast ────────────────────────────────────────────────────── */
function useToasts() {
  const [items, setItems] = useState([]);
  const push = (msg, icon = "ti-check") => {
    const id = Math.random();
    setItems((x) => [...x, { id, msg, icon }]);
    setTimeout(() => setItems((x) => x.filter((i) => i.id !== id)), 2200);
  };
  const node = (
    <div className="toast-wrap">
      {items.map((i) => (
        <div className="toast" key={i.id}><i className={"ti " + i.icon}></i>{i.msg}</div>
      ))}
    </div>
  );
  return [push, node];
}

/* ── LiveNum: briefly flashes when its value changes (sells "ao vivo") ─ */
function LiveNum({ value, className }) {
  const [flash, setFlash] = useState(false);
  const prev = useRef(value);
  useEffect(() => {
    if (prev.current !== value) {
      prev.current = value;
      setFlash(true);
      const id = setTimeout(() => setFlash(false), 650);
      return () => clearTimeout(id);
    }
    return undefined;
  }, [value]);
  return <span className={(className || "") + (flash ? " live-flash" : "")}>{value}</span>;
}

window.RKC = {
  Logo, CoverLogo, AppCard, AppRow, StatusMeta,
  SkCard, SkRow, SkBlock, Sparkline, makeSpark, Meter, meterColor, useToasts, LiveNum, StarIcon,
};

/* ── StarIcon: filled when on=true, outline when on=false ── */
function StarIcon({ on, size = 16 }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true"
      fill={on ? "currentColor" : "none"}
      stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"
      style={{ display: "block", flexShrink: 0 }}>
      <path d="M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z" />
    </svg>
  );
}
