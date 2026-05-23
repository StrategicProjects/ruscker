# Translations

Ruscker uses [Project Fluent](https://projectfluent.org/) for
internationalization. Each language has its own folder
(`pt`, `en`, `es`, `fr`) and the same set of `.ftl` files.

Files are **embedded into the binary** at build time via
`include_dir!`. You don't have to ship translations separately — a
rebuild of `ruscker-admin` is all that's needed.

## How translations are looked up

Order of precedence at request time:

1. Cookie `ruscker_locale` set by the in-app language picker.
2. The browser's `Accept-Language` header.
3. **pt-BR** (default), if the user's choice is unknown.

If a key is missing in a non-default locale, the renderer falls back
to the pt-BR value (and logs once). If a key is missing everywhere,
the placeholder `⟦key-name⟧` is rendered to surface the gap loudly.

## Adding or editing a translation

1. Open `assets/i18n/<lang>/<file>.ftl`.
2. Edit or add a line: `key-name = Translated text`.
3. Run `cargo test -p ruscker-admin` — the i18n unit tests will
   complain if you broke a `.ftl` parse.
4. Run `scripts/i18n-check.sh` to confirm key parity across all four
   languages (the same key must exist in every file).

## Fluent quick syntax

```ftl
# Simple key:
landing-title = Strategic Monitoring

# With variable:
card-updated = Updated { $date }

# With pluralization:
session-count = { $n ->
  [one] { $n } session
  *[other] { $n } sessions
}
```

Full reference: <https://projectfluent.org/fluent/guide/>.

## Adding a new language

1. Create `assets/i18n/<code>/` (use a two-letter ISO code:
   `pt`, `en`, `es`, `fr`, …).
2. Copy `pt/*.ftl` as a starting point and translate.
3. Add a `Locale` variant in `crates/ruscker-admin/src/i18n.rs` with
   its short code, BCP-47 tag, and native display name.
4. Rebuild — the language now appears in the picker.

## What we **don't** do

- We don't translate spec `display-name` / `description` —
  operators write those, and they are free-form text. If a spec
  needs to be multilingual, the operator can store a JSON object in
  `template-properties.i18n` (planned for Phase 2).
- We don't put HTML inside `.ftl` files. Use Fluent variables and
  shape the markup in the template.
