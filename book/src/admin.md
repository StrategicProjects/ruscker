# The admin panel

The admin panel is Ruscker's main advantage over editing YAML by hand.
It lives at `/admin`, is gated by a token, and needs a SQLite database
(`serve --db <file>`).

**Everything is configurable from the web UI** — every spec field
(including API, scaling, resource and lifecycle settings), the media
library, encrypted credentials, and the whole landing page. YAML
import/export exists for migration and backups, not as a requirement.

## Logging in

On first run, set `RUSCKER_ADMIN_TOKEN` (the `.deb` generates one on
install and prints it once) and browse to `/admin/login`: with no
accounts yet you're asked for the **token**, then walked through
creating the first **admin account** (username + password). After that
everyone signs in with their account at `/admin/login`. The token stays
as a **break-glass** login (`/admin/login?token=1`) so you can never be
locked out. Until a token is set, `/admin/*` returns `503` and only the
public landing + proxy are served.

The footer has the same **language** (pt-BR / en-US / es-ES / fr-FR)
and **theme** (light / dark / auto) pickers as the public portal. The
top-right corner shows your current **access level**.

## Users and access levels (roles)

Each person gets their own account (username + password). Admins manage
accounts under **Users** (`/admin/users`): create one, assign a role,
reset a password, or remove it. A new user gets an initial password you
choose and is asked — once, on first login — whether to change it.
**Password fields are masked** (type `password`) throughout the panel, so a
shoulder-surfer can't read a password as you type it.

| Role | Can do |
|---|---|
| **Viewer** | view the Dashboard only (read-only) |
| **Editor** | view + manage Apps and Media; view the Dashboard and stop/restart replicas |
| **Admin** | everything, including managing users, credentials, the landing editor, custom blocks and the audit log |

The panel shows only the sections your role can reach. Enforcement is
server-side — hiding a nav link is just UX; the routes themselves
return `403` for a role that isn't allowed. The audit log records the
acting username. A **last-admin guard** stops you deleting or demoting
the only remaining admin (so the portal can't be locked out); the
`RUSCKER_ADMIN_TOKEN` break-glass login is the other safety net.

## Screens

### Dashboard
A live view of running replicas: per-container state, uptime, sessions,
CPU and memory (refreshed over Server-Sent Events). Stop or restart a
replica; open its logs. Shows a banner when started without
`--docker`.

### Apps
The list of specs — apps, APIs and external links — with create, edit
and delete. Each row has a **featured star** next to the actions: click
it to toggle whether the app appears in the landing page's *Featured*
carousel, inline, without opening the editor (solid = featured).

The add/edit form is organised into three bands so the layout maps to
intent: **Identity** (id, name, description), a **Metadata & visibility**
band (the featured flag, access groups/users, subject, logo and cover),
and a collapsible **Advanced** band for runtime knobs. It has a type
selector, a **live card preview**, and a **logo picker** that pulls from
the media library (no need to type `/assets/img/...` by hand). A **"?"
help popover** on every field explains what it does.

Under the collapsible **Advanced** band, every remaining spec option is
editable too — so an app can be configured end-to-end from the web UI,
without touching YAML:

- **Runtime** — seats per container, session lifetime, inner container
  port and platform.
- **API** (for `type: api`) — container port, rate limit, docs/health
  paths, permissive CORS.
- **Scaling** — min/max replicas and concurrent requests per replica.
- **Resources** — per-container CPU and memory limits.
- **Lifecycle** — the heartbeat (idle-session) timeout.

Every advanced field is optional; leaving it blank keeps Ruscker's
default, so the section stays out of the way until you need it.

### Media
Upload images (PNG/JPEG → WebP), served at `/assets/img/<file>`. These
are the card logos and covers.

The gallery is a single unified library — **built-in logos** (brand marks
shipped with Ruscker) are seeded here automatically alongside your uploads.
Every image can be **deleted** from the gallery; if it is referenced by any
spec logo/cover or landing logo, the entry shows an **"in use" badge** so
you know before deleting.

When editing a spec you can open a **modal picker** (search, browse, drag
and drop, or upload inline without leaving the form) to select a logo or
cover. A "Choose image" button auto-uploads on file select for a one-click
flow.

### Credentials
A named, AES-256-GCM-encrypted store for registry credentials (needs
`RUSCKER_MASTER_KEY`). Passwords never appear in the YAML or in the
panel after saving. Each entry accepts either a literal password
(encrypted at rest) or a **pure `${VAR}` env-ref** — stored verbatim and
resolved to the real value only at container pull time.

In the spec form the Registry section is a **credential picker**: type or
select the name of a stored credential and Ruscker resolves it at spawn.
There is no need to inline registry passwords in a spec.

### Landing editor
Customise the public landing without a custom template:

- **Colours + intro** — header background/foreground, a per-locale
  intro paragraph.
- **SEO & sharing** — page title, meta description, `og:image`. The
  landing `<head>` emits `description` + `og:*` + `twitter:card`.
- **Analytics** — paste a provider snippet (Plausible, Matomo, GA…)
  and list its origins; Ruscker widens **only the landing's** CSP so
  the script can load.
- **Logos** — add logos to the landing **header** or **footer** slot.
  Each logo has its own **alignment** (left / center / right), an optional
  **click-through link**, and a **height** in pixels. Multiple logos
  sharing a slot and alignment render side by side. Images come from the
  Media library — pick one with the same modal picker used in the spec
  form.

### Blocks
Custom HTML blocks rendered in the landing `top` (after the header) and
`bottom` (after the card grid) slots. Create/edit/delete, enable/
disable, and reorder within a slot with the ↑/↓ buttons. Each block can
declare CSP origins for any third-party content it embeds.

> Block and analytics HTML is rendered **verbatim** on the public
> landing. It's admin-only input — the intentional escape hatch — so
> only paste HTML you trust.

### Audit log
Every admin mutation (spec/image/credential/landing/block changes,
imports) is recorded with actor, action, target and timestamp.

## Config vs. database

`serve --config` drives the public landing and the proxy. The admin
panel reads and writes the SQLite DB. Use `ruscker import` to populate
the DB from a YAML, and `ruscker export` to round-trip it back — both
preserve specs, landing customization, SEO/analytics and custom
blocks.
