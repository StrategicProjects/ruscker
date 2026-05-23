#!/usr/bin/env bash
# Verify that every key present in the default-locale .ftl files also
# exists in every other locale. Exits 0 if parity holds, 1 otherwise.
#
# Run from the repo root (or any subdirectory — it cd's to the root).
# Used by CI to gate PRs from drifting into untranslated keys.

set -euo pipefail

cd "$(dirname "$0")/.."

I18N_ROOT="crates/ruscker-admin/assets/i18n"
DEFAULT="pt"

if [ ! -d "$I18N_ROOT/$DEFAULT" ]; then
    echo "error: default locale dir not found: $I18N_ROOT/$DEFAULT" >&2
    exit 1
fi

# Extract keys from a .ftl file: lines matching `^key-name = ...`,
# ignoring comments and indented continuation lines.
extract_keys() {
    grep -E '^[a-zA-Z][a-zA-Z0-9_-]*[[:space:]]*=' "$1" \
        | sed -E 's/^([a-zA-Z][a-zA-Z0-9_-]*).*/\1/' \
        | sort -u
}

status=0
# Portable across BSD (macOS) and GNU find — strip the path manually.
default_files=$(find "$I18N_ROOT/$DEFAULT" -maxdepth 1 -name '*.ftl' -exec basename {} \; | sort)

for filename in $default_files; do
    default_keys=$(extract_keys "$I18N_ROOT/$DEFAULT/$filename")
    for locale_dir in "$I18N_ROOT"/*/; do
        locale=$(basename "$locale_dir")
        [ "$locale" = "$DEFAULT" ] && continue
        target="$locale_dir/$filename"
        if [ ! -f "$target" ]; then
            echo "missing file: $target" >&2
            status=1
            continue
        fi
        target_keys=$(extract_keys "$target")
        missing=$(comm -23 <(echo "$default_keys") <(echo "$target_keys") || true)
        if [ -n "$missing" ]; then
            echo "missing keys in $target:" >&2
            echo "$missing" | sed 's/^/  - /' >&2
            status=1
        fi
        extra=$(comm -13 <(echo "$default_keys") <(echo "$target_keys") || true)
        if [ -n "$extra" ]; then
            echo "extra keys in $target (not in default):" >&2
            echo "$extra" | sed 's/^/  + /' >&2
            status=1
        fi
    done
done

if [ "$status" = "0" ]; then
    echo "✓ i18n key parity OK"
fi
exit "$status"
