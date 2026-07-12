## Summary

<!-- What changed, and why? -->

## Verification

<!-- List the commands or checks used to validate the change. -->

## Migration checklist

<!-- Choose one when this PR changes database migrations. -->

- [ ] Not applicable: this PR does not add, rename, or remove a migration.
- [ ] SQLite and Postgres twins use the same filename in `migrations/` and `migrations-pg/`, and `bash ./scripts/migration-parity-check.sh` passes.
