# Design handoff — vendored reference

This directory is the **high-fidelity UX prototype** handed off for the
Ruscker redesign, vendored verbatim so it's versioned and accessible to
whoever implements it. Tracking issue: **#623**.

**It is reference, not production code.** The prototype is React/JSX with
Babel-in-browser (`Ruscker UX.html` + `src/*.jsx`); the real product is
**Askama + HTMX + Alpine.js + Tailwind 4, zero Node, no SPA**. Recreate
the screens — do not port the JSX.

## What to trust

- **`assets/ruscker.css`** — source of truth for design tokens and
  component styles. Reconcile against the live
  `crates/ruscker-admin/assets/tailwind/input.css`.
- **`README.md`** — view-by-view spec, interactions, perceived-performance
  primitives, and the shared-state model.

## Known doc drift

- The README prose mentions **Geist** for typography, but both
  `ruscker.css` and the live `input.css` use **Jost** (self-hosted, latin
  subset). Use **Jost** — ignore the Geist reference.
- Tabler icons: the subset the app serves is **outline only** — there is
  no `ti-*-filled`. "Filled" glyphs (e.g. the featured star) are inline
  SVG with a toggled `fill`, as already done in the app.
- The "simulated user" picker in the prototype is a prototype-only device
  for previewing per-group access; it is **not** part of the product
  (the real app uses login/session).

To browse the prototype: open `Ruscker UX.html` directly in a browser
(no build step — uses in-browser Babel).
