# Next steps — strategic options (for discussion)

Ruscker is feature-complete (Phases 0–7), polished, and running in
production. The "obvious" work is done; the next direction is a genuine
choice, not a backlog to grind. This document captures the options so we
can decide deliberately. It is a discussion artifact, not a commitment.

> Status update (2026-06-10): the reliability preference below was
> executed as the v0.2.5 audit (18 fixes) and the documentation
> refresh. The remaining options stand as written.
>
> Owner preference (2026-06-01): **reliability** + a **documented
> single-node deploy recipe**. The plan below leads with those; the rest
> is kept for context.

## The framing question

At this maturity the bottleneck is rarely "another operator feature".
It's one of three goals, and which one we pick sets every priority:

1. **Adoption** — take Ruscker from "great single-operator tool" to
   "adoptable by a team/org". Levers: external auth (OIDC), deploy
   ergonomics, demo polish.
2. **Correctness de-risking** — prove the core claim (Shiny / Streamlit /
   Dash work behind the proxy, not just Jupyter).
3. **Depth for a specific production** — make the existing deployment
   more reliable under real load.

The chosen direction is a blend of **(3) depth** and the deploy half of
**(1) adoption**.

---

## Chosen direction

### A. Documented single-node deploy recipe — *high ROI, low–medium effort*

Recent work surfaced real deploy friction that every adopter would hit:

- SSE behind nginx was buffered until a reverse-proxy hint
  (`X-Accel-Buffering: no`) was added — the live dashboard appeared
  frozen on a subpath mount.
- The reverse proxy serving its own root `/favicon.ico` shadows the
  app's icon on some browsers.
- The `location` for the app needs the right `proxy_buffering` /
  `Upgrade` / timeout settings for WebSockets *and* SSE.

**Deliverable:** a battle-tested, copy-pasteable recipe for the common
single-node case:

- a reference `nginx` server block (root mount and subpath mount), with
  the WebSocket + SSE + favicon footguns pre-solved and commented;
- the `systemd` unit + `EnvironmentFile` layout (master/cookie/admin
  keys), matching what the `.deb` ships;
- optionally a `docker-compose.yml` for the non-HA case (we only ship an
  HA compose under `examples/ha/` today);
- a "deploying" book chapter that walks it end-to-end and lists the
  footguns explicitly.

This makes nobody re-discover what we just debugged.

### B. Reliability depth — *medium effort, demand-driven value*

Known refinements that matter under sustained real load (see CLAUDE.md
"Known gaps"):

- **Scale-down hysteresis / post-drop cooldown.** ✅ *Shipped.* A
  `seats=1` spec with long sessions could flap; a post-drop cooldown
  (`DEFAULT_SCALE_DOWN_COOLDOWN_TICKS`) now gates the saturation respawn
  after a scale-down, on top of the existing hysteresis.
- **Per-spec heartbeat-timeout override.** A single global timeout today;
  some apps need a longer idle window than others.
- **Observability template.** The Prometheus endpoint already exists
  (`metrics.rs`); ship a ready-made Grafana dashboard JSON so operators
  get graphs for free.
- **Backoff / health on spawn failures.** Confirm spawn failures back off
  and surface clearly in the dashboard rather than silently retrying.

### C. Core-claim de-risking (cheap insurance, do alongside) — *low effort*

The whole value proposition is hosting Shiny / Streamlit / Dash, but the
URL-rewriting + WebSocket arc was validated against **Jupyter**, not
those. `ruscker-proxy/CLAUDE.md` already flags a missing
`tests/shiny_e2e.rs`. Spin up a real Shiny *and* a Streamlit container
behind the proxy and exercise the rewrite + WS pump end to end. If it
surfaces a bug, far cheaper to fix now than after adopters hit it.
Recommended regardless of the chosen direction.

---

## Deferred (noted, not chosen now)

- **External auth (Phase 8 — OIDC / OAuth / SAML / LDAP).** The single
  biggest *adoption* unlock: today auth is local accounts + a break-glass
  token, no SSO. High effort, high leverage — revisit if/when adoption
  (vs. depth) becomes the goal.
- **Demo ecosystem** (`StrategicProjects/ruscker-*-demo` forks, issues
  #389–397). First-run polish / marketing. Low effort, picks up whenever.

---

## Suggested first slice

1. **C — Shiny + Streamlit e2e** (cheap, de-risking, useful in every
   scenario).
2. **A — deploy recipe** (nginx + systemd + compose + book chapter),
   capturing the footguns we just hit.
3. **B — reliability** items as demand dictates (the scale-down
   cooldown, which had the user-visible symptom, has since shipped).

Open question to settle before slicing: is the deploy recipe targeting
**root-mount**, **subpath-mount**, or **both** as first-class? (Both is
ideal but doubles the nginx surface to document/test.)
