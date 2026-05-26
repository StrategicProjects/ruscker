# Configuration

Ruscker is configured with an `application.yml` in the ShinyProxy
schema, plus a few Ruscker-specific extensions. The full reference
below is the same document shipped in the repository
(`docs/YAML_SCHEMA.md`).

Secrets are never written in the YAML — use `${VAR}` interpolation and
set the variables in the environment (or `/etc/ruscker/ruscker.env`).

{{#include ../../docs/YAML_SCHEMA.md}}
