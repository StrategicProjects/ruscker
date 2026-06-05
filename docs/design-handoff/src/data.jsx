/* global window */
// ── Ruscker prototype data: app catalog + dashboard replicas + i18n ──

const KIND = {
  app:     { badge: "APP", b: "b-app", tint: "tint-app", color: "#06d6a0", ink: "#04553f" },
  api:     { badge: "API", b: "b-api", tint: "tint-api", color: "#06b6d4", ink: "#083344" },
  package: { badge: "PKG", b: "b-package", tint: "tint-package", color: "#26547c", ink: "#ffffff" },
  report:  { badge: "REL", b: "b-report", tint: "tint-report", color: "#ef476f", ink: "#ffffff" },
};

// Brand-ish accent + monogram used for the tinted cover placeholder
// (real product would drop the framework logo here).
const APPS = [
  { id:"shiny", name:"Shiny", kind:"app", mono:"R", accent:"#447099",
    subject:"Documentação", locked:true, status:"updated", updated:"02/06", featured:true,
    desc:{pt:"Apps web reativos em R.", en:"Reactive web apps in R."}, replicas:6, state:"ready",
    accessGroups:["admin","editor"] },
  { id:"jupyter", name:"Jupyter", kind:"app", mono:"Jy", accent:"#f37726",
    subject:"Documentação", locked:true, status:"updated", updated:"02/06", featured:true,
    desc:{pt:"Notebooks interativos servidos como app web.", en:"Interactive notebooks served as a web app."}, replicas:4, state:"ready",
    accessGroups:["admin","editor","viewer"] },
  { id:"rstudio", name:"RStudio Server", kind:"app", mono:"R", accent:"#75aadb",
    subject:"Documentação", locked:true, status:"updated", updated:"02/06", featured:true,
    desc:{pt:"O IDE RStudio no navegador, servido por sessão.", en:"The RStudio IDE in your browser, served per session."}, replicas:3, state:"ready",
    accessGroups:["admin"] },
  { id:"streamlit", name:"Streamlit", kind:"app", mono:"St", accent:"#ff4b4b",
    subject:"Documentação", locked:true, status:"updated", updated:"02/06", featured:true,
    desc:{pt:"Dashboards de dados em Python, sem front-end.", en:"Python data dashboards, no front-end needed."}, replicas:5, state:"ready",
    accessGroups:["admin","editor","viewer"] },
  { id:"dash", name:"Dash", kind:"app", mono:"Da", accent:"#0d76bf",
    subject:"Documentação", locked:true, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Apps analíticos em Python por Plotly.", en:"Analytical Python apps by Plotly."}, replicas:2, state:"ready",
    accessGroups:["editor","viewer"] },
  { id:"voila", name:"Voilà", kind:"app", mono:"Vo", accent:"#5a9e6f",
    subject:"Documentação", locked:true, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Transforma notebooks Jupyter em apps standalone.", en:"Turns Jupyter notebooks into standalone apps."}, replicas:2, state:"warn",
    accessGroups:["viewer","editor","admin"] },
  { id:"shiny-for-python", name:"Shiny for Python", kind:"app", mono:"Sh", accent:"#1a6ea8",
    subject:"Documentação", locked:true, status:"new", updated:"02/06", featured:false,
    desc:{pt:"Apps web reativos em Python — o modelo do Shiny, ecossistema do Python.", en:"Reactive web apps in Python — Shiny's model, Python's ecosystem."}, replicas:3, state:"boot",
    accessGroups:["admin","editor"] },
  { id:"quarto", name:"Quarto", kind:"app", mono:"Qo", accent:"#39729e",
    subject:"Documentação", locked:true, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Sistema de publicação científica e técnica.", en:"Scientific and technical publishing system."}, replicas:1, state:"ready",
    accessGroups:[] },
  { id:"fastapi", name:"FastAPI", kind:"api", mono:"Fa", accent:"#059487",
    subject:"Documentação", locked:false, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Framework web de alta performance para APIs em Python.", en:"High-performance web framework for Python APIs."}, replicas:3, state:"ready",
    accessGroups:[] },
  { id:"plumber", name:"Plumber", kind:"api", mono:"Pl", accent:"#1f9e89",
    subject:"Documentação", locked:false, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Transforma código R em uma API REST documentada.", en:"Turn R code into a documented REST API."}, replicas:2, state:"ready",
    accessGroups:[] },
  { id:"bokeh", name:"Bokeh", kind:"package", mono:"Bo", accent:"#22272e",
    subject:"Documentação", locked:false, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Biblioteca de visualização interativa para navegadores.", en:"Interactive visualization library for browsers."}, replicas:1, state:"ready",
    accessGroups:[] },
  { id:"rmarkdown", name:"R Markdown", kind:"report", mono:"Rm", accent:"#75aadb",
    subject:"Documentação", locked:true, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Documentos dinâmicos, relatórios e dashboards em R.", en:"Dynamic documents, reports and dashboards in R."}, replicas:1, state:"ready",
    accessGroups:["editor","viewer"] },
  { id:"ruscker-docs", name:"Ruscker", kind:"package", mono:"Ru", accent:"#0f6e56",
    subject:"Documentação", locked:false, status:"none", updated:"02/06", featured:false,
    desc:{pt:"Portal de containers e balanceador de carga para apps e APIs interativos.", en:"Container portal and load balancer for interactive web apps and APIs."}, replicas:1, state:"ready",
    accessGroups:[] },
];

// host pool for replicas
const HOSTS = ["local", "node-a", "node-b"];
function rid() { return Math.random().toString(16).slice(2, 14); }

// Build dashboard replicas grouped under each app.
function buildReplicas() {
  const groups = [];
  let total = 0;
  for (const a of APPS) {
    const n = a.replicas;
    const reps = [];
    for (let i = 0; i < n; i++) {
      const booting = a.state === "boot" && i === 0;
      const warm = a.state === "warn" && i === 0;
      reps.push({
        cid: rid(),
        host: HOSTS[i % HOSTS.length],
        state: booting ? "boot" : warm ? "warn" : "ready",
        uptime: booting ? "—" : `${(2 + i)}h ${10 + i * 7}m`,
        sessions: booting ? 0 : Math.floor(Math.random() * 6),
        sessionsMax: 10,
        cpu: booting ? 0 : warm ? 34 : Math.floor(Math.random() * 18),
        mem: booting ? 0 : warm ? 612 : 120 + Math.floor(Math.random() * 200),
      });
    }
    total += n;
    groups.push({ ...a, reps });
  }
  return { groups, total };
}

const I18N = {
  pt: {
    portal: "Portal", dashboard: "Painel",
    portalSub: "Catálogo de aplicações e APIs",
    dashSub: "Estado dos containers e sessões em tempo real",
    search: "Buscar aplicação…", searchLong: "Buscar por nome, descrição, autor…",
    all: "Todos", apps: "Aplicações", apis: "APIs", packages: "Pacotes", reports: "Relatórios",
    free: "livres", restricted: "restritos",
    recent: "Recentes", name: "Nome", sort: "Ordenar",
    featured: "Em destaque", signin: "Entrar", openDocs: "Abrir",
    results: "resultados", clear: "limpar filtros", noResults: "Nenhuma aplicação encontrada",
    noResultsSub: "Tente outro termo ou limpe os filtros.",
    loading: "Carregando catálogo…",
    categories: "Categorias", access: "Acesso", quick: "Atalhos",
    containers: "Containers", appsReplicas: "Apps com réplicas", activeSessions: "Sessões ativas",
    tracked: "Sessões rastreadas", memUsed: "Memória usada",
    activeReplicas: "Réplicas ativas", groupedByApp: "Agrupadas por aplicação",
    expandAll: "Expandir tudo", collapseAll: "Recolher tudo",
    replicas: "réplicas", replica: "réplica", state: "Estado", uptime: "Tempo no ar",
    sessions: "Sessões", cpu: "CPU", mem: "Memória", host: "Host", container: "Container", actions: "Ações",
    ready: "pronto", booting: "iniciando", warn: "alerta", stopped: "parado",
    health: "Saúde", healthy: "saudável", logs: "Logs", restart: "Reiniciar", stop: "Parar",
    refreshing: "Atualizando…", updated: "atualizado", live: "ao vivo",
    backToList: "Réplicas", filterApp: "Filtrar app…",
    avgCpu: "CPU média", totalMem: "Memória total", totalSessions: "Sessões",
    newBadge: "novo", updatedBadge: "atualizado",
    restarted: "reiniciado", peakToday: "pico hoje", lastRefresh: "atualizado agora",
  },
  en: {
    portal: "Portal", dashboard: "Dashboard",
    portalSub: "Application and API catalog",
    dashSub: "Live container and session state",
    search: "Search application…", searchLong: "Search by name, description, author…",
    all: "All", apps: "Applications", apis: "APIs", packages: "Packages", reports: "Reports",
    free: "public", restricted: "restricted",
    recent: "Recent", name: "Name", sort: "Sort",
    featured: "Featured", signin: "Sign in", openDocs: "Open",
    results: "results", clear: "clear filters", noResults: "No application found",
    noResultsSub: "Try another term or clear the filters.",
    loading: "Loading catalog…",
    categories: "Categories", access: "Access", quick: "Shortcuts",
    containers: "Containers", appsReplicas: "Apps with replicas", activeSessions: "Active sessions",
    tracked: "Tracked sessions", memUsed: "Memory used",
    activeReplicas: "Active replicas", groupedByApp: "Grouped by application",
    expandAll: "Expand all", collapseAll: "Collapse all",
    replicas: "replicas", replica: "replica", state: "State", uptime: "Uptime",
    sessions: "Sessions", cpu: "CPU", mem: "Memory", host: "Host", container: "Container", actions: "Actions",
    ready: "ready", booting: "booting", warn: "warning", stopped: "stopped",
    health: "Health", healthy: "healthy", logs: "Logs", restart: "Restart", stop: "Stop",
    refreshing: "Refreshing…", updated: "updated", live: "live",
    backToList: "Replicas", filterApp: "Filter app…",
    avgCpu: "Avg CPU", totalMem: "Total memory", totalSessions: "Sessions",
    newBadge: "new", updatedBadge: "updated",
    restarted: "restarted", peakToday: "peak today", lastRefresh: "updated just now",
  },
};

window.RK = { KIND, APPS, buildReplicas, I18N, rid };

// ── Real brand logos (official simple marks; graceful monogram fallback
//    via LogoImg). Drop exact files into assets/logos/<id>.svg to override. ──
const LOGOS = {
  jupyter: "https://cdn.simpleicons.org/jupyter",
  rstudio: "https://cdn.simpleicons.org/rstudioide",
  streamlit: "https://cdn.simpleicons.org/streamlit",
  dash: "https://cdn.simpleicons.org/plotly",
  "shiny-for-python": "https://cdn.simpleicons.org/python",
  quarto: "https://cdn.simpleicons.org/quarto",
  fastapi: "https://cdn.simpleicons.org/fastapi",
  shiny: "https://cdn.simpleicons.org/r",
  rmarkdown: "https://cdn.simpleicons.org/r",
  plumber: "https://cdn.simpleicons.org/r",
  bokeh: "https://cdn.simpleicons.org/python",
  voila: "https://cdn.simpleicons.org/jupyter",
  "ruscker-docs": "assets/ruscker-mark.svg",
};
APPS.forEach((a) => { a.logo = LOGOS[a.id] || null; });
