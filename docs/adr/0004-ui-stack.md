# ADR 0004 — UI stack: Askama + HTMX + Tailwind 4, no SPA

Status: accepted

## Context

The admin panel and landing page need to look modern, render fast, and
be maintainable. The mainstream answer in 2026 would be a React /
SvelteKit / Solid SPA talking to a Rust API. This adds:

- A Node toolchain (npm, build tooling, lockfile)
- A separate frontend codebase
- Hydration mismatches and CSR/SSR complexity
- Larger bundles, more JS for users to download

We're building a portal for a Brazilian state government and similar
institutional users. Many will deploy to modest VMs and want minimal
operational surface.

## Decision

Server-rendered templates with **Askama** (compile-time, typed),
client interactivity via **HTMX** for fragment swapping, **Alpine.js**
for small in-page state, **Tailwind 4** for styling.

No SPA, no Node build toolchain, no separate frontend service.

## Why each piece

### Askama

- Compile-time templates, type-checked against Rust structs.
- Zero runtime template parsing — fast and safe.
- Jinja-like syntax familiar to anyone who's used Django, Flask, Liquid.
- No template injection vulnerabilities possible at runtime.

### HTMX

- Lets us return HTML fragments from endpoints.
- `hx-get`, `hx-post`, `hx-swap` give us SPA-like interactivity
  declaratively in HTML attributes.
- Tiny (~14 KB), zero dependencies.
- Server stays the source of truth for rendered HTML — no client/server
  state divergence.

### Alpine.js

- For widgets that need genuinely client-side state (tabs, dropdowns,
  toggles).
- Small (~15 KB), declarative, no build step.
- Used sparingly. If something is complex enough to want Vue's
  composition API, it should be server-side via HTMX swap.

### Tailwind 4

- The standalone CLI binary builds CSS from class scans — no Node
  required.
- Tailwind 4's CSS-first config is fast and ergonomic.
- Atomic classes pair perfectly with HTMX fragments (no class drift
  between Rust template and React component).

## Consequences

### What we gain

- One language end to end (Rust).
- One deploy artifact (the binary + a static CSS file).
- No npm or webpack to break.
- Fast first paint (everything is server-rendered, including the
  shell).
- Search engines and curl can read every page.
- Trivially testable: assert against rendered HTML strings.

### What we lose

- **Complex client-side state** is harder. If we need rich
  drag-and-drop, virtualized lists, or canvas-heavy charts, we'd add
  more JS than HTMX comfortably handles. Mitigated by: most admin
  screens are forms and tables, which HTMX nails. The monitoring
  dashboard's charts use a small JS library (Chart.js or similar)
  loaded on that page only.
- **Offline-first / mobile-app feel** isn't possible. We accept this;
  Ruscker is an operator tool, not a consumer app.

### What we explicitly don't do

- **Server-side rendering of React/Vue** (Yew, Leptos, Dioxus,
  Trunk-based stacks). They're improving but pull in a complex
  WASM/build story we don't need.
- **JSON API + client SPA**. Rejected as massive overkill.

## Bundle budget

Total JS shipped to the browser per page should stay under 50 KB
minified for typical pages, 200 KB for the dashboard (with Chart.js).

If we ever exceed that, we revisit.
