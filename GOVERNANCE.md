# Governance

## Maintainers

Fetch Hive maintains this project. Reviews land through GitHub pull requests
on `Fetch-Hive/openapi-mcp`.

## Cadence

Maintainers review the issue and PR backlog at least monthly. Security
fixes can ship as patch releases on the same day.

## Versioning

This project follows Semantic Versioning.

- **0.x**: public API may change with minor bumps. Document breaking CLI
  flag and JSON schema changes in the release notes.
- **1.0+**: CLI flags, JSON `--json` schemas, and IR `ir_version` follow
  semver. Additive IR fields may bump the minor IR version (`1.1`).

Config `schema_version` is independent. A 0.x CLI that sees
`schema_version > 1` exits and points at `mcp-gateway upgrade`.

## Licence

Apache License 2.0. See `LICENSE` and `NOTICE`.
