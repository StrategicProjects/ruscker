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
- **SVG logos** are detected by file extension and rendered with
  `object-fit: contain` automatically, so the intentional whitespace
  around a mark is preserved instead of being cropped.
- Card covers render with `alt=""` (treated as decorative — the visible
  title alongside carries the semantic meaning), so screen readers don't
  announce a redundant filename.
- SVGs are loaded via `<img src=...>`, **not** inlined — this
  blocks SVG-script-tag XSS from operator uploads.

## Caching

| Asset class | Strategy |
|---|---|
| **Versioned bundle assets** (`?v=<version>` on CSS/JS/built-in art) | `cache-control: public, max-age=31536000, immutable` — the `?v=` query busts the cache on every release, so the bytes behind a given URL never change. |
| **Uploaded / dynamic images** (Media library, card covers) | Short `max-age` plus an `ETag`; a reload sends `If-None-Match` and gets a cheap `304` when unchanged. Editing an image changes its content hash, so the next request re-fetches. |

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

## Example config and images

`examples/application.yml` references a couple of `/assets/img/...`
cover paths, but the image files themselves are **not** bundled in the
repository (the originals came from a real install and were sanitized
out). Running `ruscker serve --config examples/application.yml` renders
the portal fine — those cards simply fall back to their tint/monogram
cover. To see real covers, drop your own files in an `assets/img/`
folder beside the config (or point `--images-dir` at one), or upload
them in the admin **Media** library.

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
