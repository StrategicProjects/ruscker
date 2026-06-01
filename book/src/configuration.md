# Configuration

Ruscker is configured with an `application.yml` in the ShinyProxy
schema, plus a few Ruscker-specific extensions. The full reference
below is the same document shipped in the repository
(`docs/YAML_SCHEMA.md`).

Secrets are never written in the YAML — use `${VAR}` interpolation and
set the variables in the environment (or `/etc/ruscker/ruscker.env`).

## Quick references

A few cross-cutting concerns have dedicated chapters and field sets:

- **Subpath mounting** (`server.context-path` / `--base-path`) — when
  you can't dedicate a subdomain and need the portal under
  `example.org/apps/`. See [Mounting under a base path][base-path] in
  the deploy guide and [`server.context-path`](#servercontext-path--subpath-mounting)
  in the schema below.
- **Per-user access** (`access-groups` / `access-users`) — restrict
  which users see and reach each spec. See
  [Per-user access](#per-user-access) below and
  [`Spec.access-groups`](#containerized-specs-shiny-streamlit-dash-voilà-api)
  in the schema. Group membership is set per user in the admin users
  page.
- **Active-active HA** — when running more than one Ruscker instance
  behind a load balancer, point them at a shared Postgres for the
  **sign-in session** so any node can serve any authenticated request;
  if you can't, pin the session-bearing paths to one upstream. See
  [Shared admin sessions][ha-sticky] in the deploy guide.
- **Branding, logos, SEO, analytics, custom HTML** — header/footer
  logos, colors, SEO/social meta, analytics snippets, and custom HTML
  blocks. See [`proxy.landing-customization`](#proxylanding-customization)
  below.
- **Named registry credentials** (`docker-registry-credential`) — store
  registry passwords encrypted in the admin panel and reference them by
  name instead of writing credentials inline. See
  [Registry credentials](#containerized-specs-shiny-streamlit-dash-voilà-api)
  in the schema below.

[base-path]: ./deploying.md#4b-mounting-under-a-base-path-subpath
[ha-sticky]: ./deploying.md#shared-admin-sessions-eliminate-the-sticky-upstream-caveat

## Per-user access

`access-groups` / `access-users` on a spec scope who can **see** the
card on the landing **and** reach the upstream at `/app` / `/api`. A
spec with neither key is **open** — visible to everyone, including
anonymous visitors. Otherwise:

- An **admin** session sees everything.
- A **signed-in user** sees a restricted spec when their username is
  in `access-users` *or* one of their groups is in `access-groups`.
- An **anonymous visitor** only sees open specs.

Enforcement is real — the `/app` and `/api` guards reject unauthorized
requests (anonymous on `/app` → redirected to login; otherwise 403),
not just hide the landing card.

Group membership is per-user and lives in the database. Set it in the
admin **Users** page (groups column, inline edit). The same user
record drives both portal visibility and admin role (Admin / Editor /
Viewer) — see [The admin panel](./admin.md).

Example:

```yaml
proxy:
  specs:
    - id: open-app
      display-name: Open App
      container-image: demo/img        # no access keys ⇒ open
    - id: analysts-app
      display-name: Analysts App
      container-image: demo/img
      access-groups: [analysts]
    - id: vip-app
      display-name: VIP App
      container-image: demo/img
      access-users: [carol]
```

The full schema, including every other Ruscker-specific extension,
follows.

{{#include ../../docs/YAML_SCHEMA.md}}
