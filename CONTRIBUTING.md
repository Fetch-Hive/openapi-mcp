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

Optional Phase 1 corpus lives under `fixtures/corpus/`. Add a case to
`fixtures/corpus/MANIFEST.toml` and a corresponding OpenAPI file. Run:

```bash
mcp-gateway corpus
# or
mcp-gateway corpus --only CASE_ID
```

Do not commit live API keys. Specs used as fixtures must be public documents
or trimmed excerpts.

## Pull requests

- This repository is the single-tenant CLI and libraries. Do not add
  multi-tenant control-plane, quota, or dashboard glue here.
- Do not weaken the default SSRF policy. Private-network access is
  `--allow-private-networks` behind the `self-host` Cargo feature.

## Release

Before you commit a version bump:

```bash
./scripts/bump-version.sh 0.5.0
```

That sets `[workspace.package] version`, rewrites workspace crate versions in
`Cargo.lock`, and retags `ghcr.io/fetch-hive/mcp-gateway` in `README.md`,
`docker/compose.yaml`, `docker/cloud-init.yaml`, and `render.yaml`. Then commit,
push `main`, tag `vX.Y.Z` (cargo-dist builds the GitHub Release), and publish
the matching GHCR image. `./scripts/bump-version.sh --dry-run 0.5.0` prints the
plan without writing.
