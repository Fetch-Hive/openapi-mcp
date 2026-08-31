# Connect your API to Cursor, Codex, Claude Code

This is the point of `mcp-gateway`: OpenAPI in, MCP tools out, paste into an
editor or agent.

Use it in **both** places:

| | Upstream | Typical flags |
|---|---|---|
| **WIP / branch** | `http://127.0.0.1:3000` (or Docker RFC1918) | `--allow-private-networks` and `--insecure-http` |
| **Live** | `https://api.example.com` | defaults |

The editor talks to the gateway (`http://127.0.0.1:8787/mcp` locally, or
`https://your-host/mcp` after deploy). The gateway talks to **your** API.

```bash
mcp-gateway inspect NAME --client cursor|claude-code|codex|vscode|claude|chatgpt
```

Each command prints **where to paste** and a ready fragment. Set
`MCP_GATEWAY_TOKEN` in the environment first (`init` prints it once).

| `--client` | Paste into | Transport |
|---|---|---|
| `cursor` | `.cursor/mcp.json` or Cursor Settings → MCP | HTTP |
| `claude-code` | `.mcp.json` or `claude mcp add --transport http` | HTTP |
| `codex` | `~/.codex/config.toml` or project `.codex/config.toml` | HTTP |
| `vscode` | `.vscode/mcp.json` | HTTP |
| `claude` | Claude Desktop `mcpServers` | stdio (`serve --stdio`) |
| `chatgpt` | ChatGPT custom connector | HTTPS URL + bearer |

Vendor shapes change. Cursor wants `"type": "http"` (not `streamable-http`).
Codex uses `url` + `bearer_token_env_var`. OAuth is not implemented.

After a PaaS deploy, take the same snippet and replace the URL with
`https://<your-host>/mcp`.

## Raw HTTP (no MCP client)

The Streamable HTTP transport is MCP protocol `2026-07-28` and stateless.
Every JSON-RPC POST except `initialize` must send `MCP-Protocol-Version`,
`Mcp-Method`, and `params._meta` protocol metadata. A request that omits
those headers returns HTTP 400 / JSON-RPC `-32020`.

Default bind is `http://127.0.0.1:8787/mcp` with bearer
`Authorization: Bearer $MCP_GATEWAY_TOKEN`.

Initialize (header optional; if present it must match `params.protocolVersion`):

```bash
curl -sS -D - http://127.0.0.1:8787/mcp \
  -H "Authorization: Bearer $MCP_GATEWAY_TOKEN" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2026-07-28" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```

`tools/list` (standalone; no prior initialize required):

```bash
curl -sS -D - http://127.0.0.1:8787/mcp \
  -H "Authorization: Bearer $MCP_GATEWAY_TOKEN" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2026-07-28" \
  -H "Mcp-Method: tools/list" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"curl","version":"0"}}}}'
```

Successful responses are JSON (`json_response = true`), not SSE.
