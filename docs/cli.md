# CLI reference

Generated from clap. Hidden Phase 1 aliases (`compile`, `list-tools`, `call`,
`corpus`) are listed by `mcp-gateway --help-all`.

```text
mcp-gateway init [--force] [--bind ADDR] [--allow-private-networks]
mcp-gateway add-spec --name NAME (--url HTTPS_URL | --file PATH)
mcp-gateway list [--json]
mcp-gateway inspect [NAME] [--tool TOOL] [--client cursor|claude|vscode|chatgpt]
mcp-gateway auth add NAME --type none|bearer|basic|api_key_header|api_key_query|custom_headers
             (--from-env VAR | --from-file PATH)
mcp-gateway auth list [NAME]
mcp-gateway auth remove NAME
mcp-gateway serve NAME [--stdio | --bind ADDR] [--path /mcp] [--expose] [--allow-anonymous]
mcp-gateway doctor [NAME] [--offline] [--json]
mcp-gateway test NAME TOOL [--args JSON] [--timeout SECS]
mcp-gateway logs [--since RFC3339] [--tool TOOL]
mcp-gateway version [--json]
mcp-gateway upgrade [--version X.Y.Z] [--dry-run]
```

`inspect` with no name (or `inspect config`) prints the config path.
`inspect NAME` (no `--tool`) lists compiled MCP tool names as a table. Names are
snake_cased from the OpenAPI `operationId` (`getInventory` → `get_inventory`),
not the literal id. `inspect NAME --tool TOOL` and `mcp-gateway test NAME TOOL`
require that compiled name.
`--follow` on `logs` is not implemented and exits 1.

Global flags: `--config PATH`, `-v`/`--verbose`, `-q`/`--quiet`, `--json`,
`--color auto|always|never`, `--allow-private-networks`.

Exit codes: `0` ok, `1` usage/config, `2` policy/SSRF/doctor-fail,
`3` supply-chain, `4` upstream/`isError`, `130` SIGINT.
