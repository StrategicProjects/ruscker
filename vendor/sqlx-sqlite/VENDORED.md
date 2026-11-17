# Vendored `sqlx-sqlite` — why this directory exists

This is `sqlx-sqlite` **0.9.0** exactly as published on crates.io, with
**one line changed** in `Cargo.toml`: the version bound on
`libsqlite3-sys`.

```diff
 [dependencies.libsqlite3-sys]
-version = ">=0.30.1, <0.38.0"
+version = ">=0.39.0, <0.41.0"
```

No source file differs. The published crate's checksum, for comparison:

```
sha256  488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c  sqlx-sqlite-0.9.0.crate
```

(`Cargo.lock`, `.cargo_vcs_info.json` and `.cargo-ok` from the package
were not copied; `Cargo.toml.orig` is kept for reference.)

## Why

SQLite before 3.54.0 can return uninitialized page-cache memory when it
opens a database together with a crafted `-wal` file (CVE-2026-119375,
RUSTSEC-2026-0459). `libsqlite3-sys` fixed it in 0.39 by bundling
SQLite 3.54, but `sqlx-sqlite` 0.9.0 — the latest sqlx — caps
`libsqlite3-sys` at `<0.38.0`, so cargo cannot select a fixed release.

Lifting that cap here lets cargo use the real `libsqlite3-sys` from
crates.io (checksummed, updatable with `cargo update`), and `Cargo.lock`
states the fixed version — so `cargo audit` needs no exception.

An earlier arrangement vendored `libsqlite3-sys` 0.37.0 with newer
SQLite sources swapped in. It fixed the bug but left `Cargo.lock`
saying `0.37.0`, which needed an `ignore` in `.cargo/audit.toml`. This
replaces it: a smaller copy (a pure-Rust driver, not 9 MB of C), an
unmodified upstream SQLite bundle, and no ignored advisory.

## What keeps it honest

* The test `db::bundled_sqlite::bundled_sqlite_is_patched` asks the linked
  library for its version and fails the gate below 3.54.0.
* The whole test suite runs against this driver with the newer
  `libsqlite3-sys`; the lower bound is `0.39.0` so cargo can never fall
  back to a release with the bug.

## How to remove it

When a sqlx release accepts `libsqlite3-sys >= 0.39`: upgrade sqlx,
delete this directory and the `[patch.crates-io]` section of the
workspace `Cargo.toml`, and run `cargo update -p libsqlite3-sys`.
