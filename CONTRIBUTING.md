# Contributing

## Developer Certificate of Origin

By contributing, you agree to the [Developer Certificate of Origin](https://developercertificate.org/)
(DCO). Sign each commit with `Signed-off-by: Your Name <you@example.com>`
(`git commit -s`).

## Build and test

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

## Compile corpus

Optional corpus lives under `fixtures/corpus/`. Add a case to
`fixtures/corpus/MANIFEST.toml` and a corresponding OpenAPI file. Run:

```bash
mcp-gateway corpus
# or
mcp-gateway corpus --only CASE_ID
```

Do not commit live API keys. Specs used as fixtures must be public documents
or trimmed excerpts.

## Pull requests

- This repository is the CLI and libraries. Quota, dashboard, and
  multi-tenant APIs are out of scope.
- Do not weaken the default SSRF policy. Private-network access is
  `--allow-private-networks` behind the `self-host` Cargo feature.
