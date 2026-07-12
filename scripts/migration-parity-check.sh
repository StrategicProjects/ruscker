#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sqlite_dir="$repo_root/crates/ruscker-admin/migrations"
postgres_dir="$repo_root/crates/ruscker-admin/migrations-pg"

for dir in "$sqlite_dir" "$postgres_dir"; do
    if [[ ! -d "$dir" ]]; then
        printf 'migration parity: directory not found: %s\n' "$dir" >&2
        exit 1
    fi
done

migration_names() {
    find "$1" -maxdepth 1 -type f -name '*.sql' -exec basename {} \; | LC_ALL=C sort
}

if ! diff -u <(migration_names "$sqlite_dir") <(migration_names "$postgres_dir"); then
    printf '%s\n' \
        'migration parity failed: SQLite and Postgres must contain the same .sql filenames.' \
        'Add, rename, or remove the matching migration in both directories.' >&2
    exit 1
fi

count="$(migration_names "$sqlite_dir" | wc -l | tr -d '[:space:]')"
printf 'migration parity ok: %s matching files\n' "$count"
