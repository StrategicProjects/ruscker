# Card images

Rules for the images shown on landing-page cards. Keep this in sync
with the runtime behavior in `crates/ruscker-admin/src/routes/`.

## Where they go

| Phase | Storage | URL served at |
|---|---|---|
| **Phase 1** | Filesystem dir, configured via `ruscker serve --images-dir <path>`. Default: `<config-dir>/assets/img/` (matches the ShinyProxy `templates/mlk/assets/img/` layout). | `GET /assets/img/<file>` via `ServeDir` |
| **Phase 2+** | SQLite blob via the admin "image library" (upload UI). Operator uploads, ruscker re-encodes to WebP and stores. | Same URL (handler reads from DB) |

The URL stays at `/assets/img/<file>` across phases so:
- ShinyProxy YAML configs (`template-properties.logo:
  "/assets/img/foo.png"`) work unchanged
- Templates don't have to know which storage backend is live

## What the YAML references

```yaml
- id: sales-dashboard
  template-properties:
    logo: "/assets/img/sales-dashboard.png"   # ← the URL the browser fetches
```

The leading `/assets/img/` is required (it's a URL path, not a
filesystem path). The bit after is whatever filename you pick.

## Format and size

| Property | Rule |
|---|---|
| **Formats** | `.png`, `.jpg` / `.jpeg`, `.webp`, `.svg` |
| **Recommended** | **WebP** at quality ~80 (best ratio of size vs. quality for photos and screenshots) |
| **Target resolution** | **800 × 400** (2:1 ratio — matches the `.rcover` slot which renders ~340 × 108 on a typical 3-col grid) |
| **Minimum** | 320 × 160 — anything smaller is visibly blurred on Retina screens |
| **Maximum file size** | Soft limit **500 KB**. `ruscker validate` warns above this; large images slow the landing for everyone |

## Behavior when an image is missing

| Situation | What renders |
|---|---|
| `template-properties.logo` is absent | Tint-only cover (the type color at 60% opacity) + small label with `spec.id` as fallback text |
| `logo` set but file 404s at runtime | Same as absent + `tracing::warn!` log line so operators see the broken link |
| Image dir not mounted at all (`--images-dir` not set and default doesn't exist) | Same as absent; no warning (this is the expected dev case) |

## Rendering

- `object-fit: cover` by default — the image fills the slot, centered.
  Good for screenshots, photos, and full-bleed art.
- For **SVG logos** with intentional whitespace around them, use
  `object-fit: contain` (planned: detect via file extension).
- `alt` text comes from `template-properties.alt` (planned for
  Phase 2). For Phase 1, cards render `alt=""` (treated as
  decorative — the title alongside provides the semantic context).
- SVGs are loaded via `<img src=...>`, **not** inlined — this
  blocks SVG-script-tag XSS from operator uploads.

## Caching

| Phase | Strategy |
|---|---|
| **Phase 1 (now)** | `cache-control: public, max-age=0, must-revalidate`. Browsers cache but always revalidate on reload — minimal surprise for the operator iterating on YAML. |
| **Phase 5 (planned)** | Hash-bearing URLs (`/assets/img/sales-dashboard-<sha>.png`) served back with `immutable` so production deployments get long-lived caching without losing the cache-bust on updates. |

> **Important:** never use `cache-control: immutable` on a URL whose
> bytes can change. Chrome interprets `immutable` as "don't check on
> reload — ever" and the user will see stale content forever.

## Adding a new card image (operator workflow)

### Phase 1 (filesystem)

```bash
# 1. Put the file alongside your config
cp my-app-cover.webp /etc/ruscker/assets/img/my-app.webp

# 2. Reference it in application.yml
#    template-properties:
#      logo: "/assets/img/my-app.webp"

# 3. Reload the page in the browser — no restart needed unless the
#    YAML structure changed
```

No `ruscker restart` required for image-only changes — `ServeDir`
re-reads from disk on every request.

### Phase 2+ (admin UI)

1. Open `/admin/images` → drag-drop the file.
2. Ruscker re-encodes to WebP, returns a stable URL.
3. Pick the image from a gallery dropdown when editing the spec.

## Bundled examples

The repository ships ~57 real card images at
`examples/assets/img/` (extracted from the SEPE / Monitoramento
Estratégico ShinyProxy install, sanitized). They cover every spec
in `examples/application.yml` — running `ruscker serve --config
examples/application.yml` shows the portal with all images visible.

## What we explicitly don't do

- **No on-the-fly resizing.** The image you upload is the image
  that ships. Phase 2's image library does one conversion to WebP
  at upload time; that's it. Image-magic-style URL parameters
  (`?w=800`) are not supported.
- **No CDN fallback.** Self-hosted only — production deployments
  put a CDN in front of the whole admin if they want one.
- **No directory listings.** `ServeDir` returns 404 for
  `/assets/img/` (the bare directory). Knowing a filename is the
  only way in.
