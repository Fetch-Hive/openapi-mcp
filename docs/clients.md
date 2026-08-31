# Client snippets

`mcp-gateway inspect NAME --client cursor|claude|vscode|chatgpt` prints a
paste-ready fragment.

**Re-verify against live vendor docs before each release.** Cursor currently
needs `"type": "http"` (not `streamable-http`) for CLI. Claude Desktop uses
stdio `command`/`args`. VS Code Copilot MCP uses `servers`. ChatGPT custom
connectors take a URL plus a bearer header. This phase does not implement
OAuth.

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

