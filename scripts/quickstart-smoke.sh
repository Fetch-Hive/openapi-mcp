#!/usr/bin/env bash
# Five-minute local/CI smoke. Uses a fixture spec so it does not depend on a
# published GitHub Release or the public internet OpenAPI fetch.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${MCP_GATEWAY_BIN:-}"
if [[ -z "${BIN}" ]]; then
  if command -v mcp-gateway >/dev/null 2>&1; then
    BIN="$(command -v mcp-gateway)"
  else
    (cd "$ROOT" && cargo build -q -p mcp-gateway-cli)
    BIN="$ROOT/target/debug/mcp-gateway"
  fi
fi
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
CFG="$WORKDIR/config.toml"
SPEC="$ROOT/crates/mcp-gateway-cli/tests/fixtures/tiny.yaml"
"$BIN" version
"$BIN" --config "$CFG" init
"$BIN" --config "$CFG" add-spec --name demo --file "$SPEC"
"$BIN" --config "$CFG" list
"$BIN" --config "$CFG" inspect demo
"$BIN" --config "$CFG" doctor demo --offline || true
echo "quickstart-smoke: ok"
