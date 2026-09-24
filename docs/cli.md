# CLI reference

Generated from clap. Hidden aliases (`compile`, `list-tools`, `call`,
`corpus`) are listed by `mcp-gateway --help-all`.

```text
mcp-gateway init [--force] [--bind ADDR] [--allow-private-networks]
mcp-gateway add-spec --name NAME (--url HTTPS_URL | --file PATH) [--base-url URL] [--insecure-http]
mcp-gateway list [--json]
mcp-gateway inspect [NAME] [--tool TOOL] [--client cursor|claude-code|codex|vscode|claude|chatgpt]
mcp-gateway auth add NAME --type none|bearer|basic|api_key_header|api_key_query|custom_headers
             (--from-env VAR | --from-file PATH)
mcp-gateway auth list [NAME]
mcp-gateway auth remove NAME
mcp-gateway serve NAME [--stdio | --bind ADDR] [--path /mcp] [--expose] [--allow-anonymous] [--tunnel] [--tunnel-auth token|public] [--base-url URL] [--url HTTPS_URL]
mcp-gateway doctor [NAME] [--offline] [--json]
mcp-gateway test NAME TOOL [--args JSON] [--timeout SECS] [--base-url URL]
mcp-gateway logs [--since RFC3339] [--tool TOOL]
mcp-gateway version [--json]
mcp-gateway upgrade [--version X.Y.Z] [--dry-run]
```

`inspect` with no name (or `inspect config`) prints the config path.
`inspect NAME` (no `--tool`) lists compiled MCP tool names as a table. Names are
snake_cased from the OpenAPI `operationId` (`getInventory` → `get_inventory`),
not the literal id. `inspect NAME --tool TOOL` and `mcp-gateway test NAME TOOL`
require that compiled name. `inspect NAME --client …` prints a paste-ready
snippet and where to put it ([clients](clients.md)).

Relative OpenAPI `servers` URLs (Petstore's `/api/v3`) are resolved against the
spec document URL when you `add-spec --url`. Already-cached IR is resolved the
same way at `test`/`serve` if the spec entry still has `url`. For a local file
whose `servers` entry is relative, pass `--base-url` on `add-spec`, `test`, or
`serve` (for a local API: `--base-url http://127.0.0.1:3000` plus
`--allow-private-networks` and `--insecure-http`). `mcp-gateway test` prints the
resolved upstream URL. A 5xx is the remote API (the public Petstore `getInventory` demo often
500s); 401/403 points at `mcp-gateway auth list`.
`--follow` on `logs` is not implemented and exits 1.

On Heroku, Render, and DigitalOcean App Platform, omit `--bind` and set `PORT`
(the platform injects it). `serve` binds `0.0.0.0:$PORT` and enables `--expose`.
If the spec is not in config, pass `--url` or set `MCP_GATEWAY_SPEC_URL` to an
HTTPS OpenAPI document. Set `MCP_GATEWAY_TOKEN`. See [deploy](deploy/README.md).

Global flags: `--config PATH`, `-v`/`--verbose`, `-q`/`--quiet`, `--json`,
`--color auto|always|never`, `--allow-private-networks`.

Exit codes: `0` ok, `1` usage/config, `2` policy/SSRF/doctor-fail,
`3` supply-chain, `4` upstream/`isError`, `130` SIGINT.

## Tunnel

`mcp-gateway serve NAME --tunnel` keeps the local server on loopback and opens
an outbound WebSocket to `wss://connect.mcp.fetchhive.com/v1/tunnel` (override
with `MCP_GATEWAY_RELAY_URL` or `[tunnel] relay_url` in config). The banner
prints `https://<slug>.mcp.fetchhive.com/mcp`. That URL is anonymous and is
released 30 minutes after the CLI disconnects. `--tunnel` cannot be combined
with `--stdio`. `--name` is hidden and exits with "persistent names are not
available yet".

`--tunnel-auth token` (the default) requires `MCP_GATEWAY_TOKEN` or
`--token-file`. Remote clients must send that bearer token.
`--tunnel-auth public` requires `--allow-anonymous` and prints a warning that
anyone who has the URL can call the server. The tunnel banner does not print
the hosted-login line. `--json` emits `{"event":"tunnel",...}` lines.
Ctrl-C exits 130 after the WebSocket closes.

`MCP_GATEWAY_RELAY_URL` overrides the relay. `[tunnel] relay_url` in
config is the fallback. The default is
`wss://connect.mcp.fetchhive.com/v1/tunnel`. An empty token in token mode
exits 1 and names `MCP_GATEWAY_TOKEN`, `--token-file`, or
`--tunnel-auth public`. `--name` exits 1 before any socket opens.

A terminal `Rejected` exits 2 for `unauthorized` and `plan_limit`, 1 for
`version_unsupported` and `name_taken` / `name_invalid` / `name_reserved`,
and 4 for every other code. Stderr is `tunnel rejected (<code>): <message>`.
`reclaim_expired`, `reclaim_invalid`, `rate_limited`, and `maintenance` do
not exit. They dial again. Ctrl-C exits 130.

The client waits 10 seconds for `Welcome`. Retry delays, the lease key, and
the public status bodies are in [Tunnel](tunnel.md). Framing and close
codes are in [Tunnel protocol](tunnel-protocol.md).
