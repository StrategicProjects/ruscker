# Vendored `libsqlite3-sys` — why this directory exists

This is `libsqlite3-sys` **0.37.0** exactly as published on crates.io,
with one change: the bundled SQLite sources were replaced by those
shipped in `libsqlite3-sys` **0.39.1**, i.e. **SQLite 3.54.1**.

## Why

SQLite before 3.54.0 can return uninitialized page-cache memory when it
opens a database together with a crafted `-wal` file
(CVE-2026-119375, RUSTSEC-2026-0459). `libsqlite3-sys` fixed it in 0.39
by bundling SQLite 3.54, but `sqlx-sqlite` 0.9.0 — the latest sqlx —
declares `libsqlite3-sys = ">=0.30.1, <0.38.0"`, so the fixed crate
cannot be selected. The workspace `Cargo.toml` therefore points
`[patch.crates-io]` at this copy.

## What differs from upstream 0.37.0

| Path | Source |
|---|---|
| `sqlite3/sqlite3.c`, `sqlite3.h`, `sqlite3ext.h` | `libsqlite3-sys` 0.39.1 (SQLite 3.54.1) |
| `sqlite3/bindgen_bundled_version.rs`, `bindgen_bundled_version_ext.rs` | `libsqlite3-sys` 0.39.1 (bindings generated for 3.54.1) |
| `sqlcipher/` | removed (only used by the `sqlcipher` features, which this workspace does not enable) |
| everything else (`build.rs`, `src/`, `Cargo.toml`, …) | `libsqlite3-sys` 0.37.0, unmodified |

Both crates were taken from the crates.io registry (checksummed by
cargo). `sqlite3/sqlite3.c` here is byte-identical to the one in
`libsqlite3-sys` 0.39.1:

```
sha256  0c657666d819f261cc4007526ad57e1669de9949df7a8ea847aa284a2bf0ee52  sqlite3/sqlite3.c
```

A test (`db::tests::bundled_sqlite_is_patched`) fails if the SQLite
actually linked into the binary is older than 3.54.0.

## How to remove it

When a sqlx release accepts `libsqlite3-sys >= 0.39`:

1. delete the `[patch.crates-io]` section from the workspace
   `Cargo.toml`;
2. delete this directory;
3. `cargo update -p libsqlite3-sys` and confirm `cargo audit` is clean
   and the test above still passes.
