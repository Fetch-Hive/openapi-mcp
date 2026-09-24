# Tunnel

Develop locally. Connect anywhere.

`mcp-gateway serve NAME --tunnel` keeps the MCP server on loopback and opens
one outbound WebSocket to the Fetch Hive relay. Remote clients call
`https://<slug>.mcp.fetchhive.com/mcp`. Your machine accepts no inbound
connection.

The CLI and the wire protocol are open source. The production relay
(`connect.mcp.fetchhive.com`, certificates for `*.mcp.fetchhive.com`) is
Fetch Hive infrastructure. Point the CLI at another relay with
`MCP_GATEWAY_RELAY_URL`. Frame types live in
[Tunnel protocol](tunnel-protocol.md).

This release is anonymous only. There is no `mcp-gateway login`, no
`tunnels` command, and `--name` exits with "persistent names are not
available yet".

## 30-second quickstart

```bash
mcp-gateway init
mcp-gateway add-spec --name petstore --url https://petstore3.swagger.io/api/v3/openapi.json
export MCP_GATEWAY_TOKEN=…    # the value `init` printed once
mcp-gateway serve petstore --tunnel
```

The banner prints a URL like `https://acrpzzs3.mcp.fetchhive.com/mcp`. Leave
the process running. From another terminal:

```bash
curl -sS -X POST "https://SLUG.mcp.fetchhive.com/mcp" \
  -H "Authorization: Bearer $MCP_GATEWAY_TOKEN" \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

Replace `SLUG` with the slug from the banner. The body matches
`curl http://127.0.0.1:8787/mcp` with the same token and the same JSON.

`mcp-gateway inspect petstore --client cursor` prints a localhost snippet
and a line telling you to swap in the tunnel URL. Paste that snippet into
the client and change only the URL.

## Auth

`--tunnel-auth token` is the default. Remote clients must send
`Authorization: Bearer $MCP_GATEWAY_TOKEN`, the same token the local server
already requires. The relay does not read or store that token. A request
with no `Authorization` header is `401`, `WWW-Authenticate: Bearer`, and

```json
{"jsonrpc":"2.0","error":{"code":-32000,"message":"missing authorization"},"id":null}
```

That response does not advertise an OAuth discovery URL, and it does not
wake your tools.

The loopback listener and the tunnel use two routers from the same handler.
The listener keeps whatever `--allow-anonymous` you passed. The tunnel
router turns anonymous access on only for `--tunnel-auth public`. In token
mode a request that arrives through the relay still needs the bearer, even
though the relay's connection is local to the CLI.

`--tunnel-auth public` requires `--allow-anonymous`. Stderr prints
`warning: this tunnel URL is reachable by anyone on the internet with no token`.
Use that only for a throwaway demo.

## What the URL is

The slug is eight characters. Each one is chosen independently from
`abcdefghjkmnpqrstuvwxyz23456789`: letters and digits, with `0`, `o`, `1`,
`l`, and `i` left out. A draw can still be all letters. That list is 31
symbols, so there are 31^8 (about 853 billion) possible slugs.

The production relay keeps one lease per slug: Redis `SET NX` on
`mcp_tunnel:lease:<slug>`. The value holds the hex SHA-256 of the 32 raw
bytes, after the base64url form is decoded. It does not hold the secret. If that key already exists, the relay draws
another slug, up to eight times, then sends `Rejected` with code
`maintenance`, message `could not allocate a tunnel name`, and
`retry_after_secs` 5. The CLI treats `maintenance` as retryable and waits
1 second.

Anonymous draws do not skip the reserved words. `internal` is eight letters
in the alphabet, so it can be issued. `connect` is seven characters, so the
WebSocket host cannot be issued as a slug. Those reserved words apply when
a named hostname exists; this release has no named hostnames.

Only one label is a tunnel host, and the relay lowercases it before the
lookup. `connect.mcp.fetchhive.com` is the relay socket.
`mcp.fetchhive.com` stays the gateway. A name with an extra dot, such as
`a.b.mcp.fetchhive.com`, is ignored. `AbCdEfGh` and `abcdefgh` are the same
lease.

The reclaim secret is 32 random bytes, encoded base64url without padding
(43 characters). `Welcome` sends it once. The CLI keeps it in memory. It is
not printed, and nothing about the tunnel is written to `config.toml`. A
new process gets a new slug.

The URL is released 30 minutes after the CLI disconnects
(`lease_grace_secs` 1800). While the lease still exists and no socket is
attached, `POST /mcp` is `503` with `Retry-After: 10` and JSON-RPC code
`-32001` (`MCP endpoint offline`). `GET /health` is always answered by the
relay: `{"slug":"<slug>","online":true}` or `"online":false`.

A connected anonymous session is recycled after 8 hours
(`max_session_secs` 28800). The relay sends `go_away` with
`reconnect_after_secs: 0`. The CLI finishes in-flight calls, reconnects,
and reclaims, so the slug stays.

Other anonymous limits in `Welcome.limits`: 1 MiB request body, 16 in-flight
requests, 60 requests per minute, 60 second request timeout. The 17th
in-flight call is answered by the CLI with `429`, `Retry-After: 1`, and
JSON-RPC code `-32000` (`too many in-flight requests`). Your API is not
called. The production relay also allows 10 new anonymous tunnels per hour
per client IP (SHA-256 of the address, key
`mcp_tunnel:create:<hash>:<unix hour>`). Over that cap the relay sends
`rate_limited` with `retry_after_secs` 3600 and the message
`too many anonymous tunnels from this network`.

Before the local router sees a tunneled request, the CLI drops `Host` and
`Content-Length`, drops hop-by-hop headers and `Cookie`, and forwards
`Authorization`. It sets `Host` to the loopback authority (`127.0.0.1:<port>`
when the bind address is unspecified). A public path of `/mcp` is rewritten
to the path you configured. `X-Forwarded-Host` still carries the public name.

Override the relay with `MCP_GATEWAY_RELAY_URL` or, lower priority, this
stanza (`config.toml.example`):

```toml
[tunnel]
relay_url = "wss://connect.mcp.fetchhive.com/v1/tunnel"
```

## Reconnect

A dropped socket dials again. The banner line is `reconnecting tunnel…`.
`--json` prints one object per state change:

```json
{"event":"tunnel","state":"connecting"}
{"event":"tunnel","state":"connected","url":"https://SLUG.mcp.fetchhive.com/mcp","slug":"SLUG"}
{"event":"tunnel","state":"reconnecting"}
{"event":"tunnel","state":"rejected","code":"unauthorized","message":"…"}
{"event":"tunnel","state":"stopped"}
```

| What happened | Next dial |
|---|---|
| Socket dropped, lease still held | `Hello` includes the reclaim secret. Same slug. Delay is 1s, then 2, 4, 8, 16, 32, 60. Each delay is multiplied by a random factor in [0.8, 1.2). |
| `go_away` | In-flight calls finish, then the CLI waits exactly `reconnect_after_secs` (no jitter) and reclaims. |
| `reclaim_expired` or `reclaim_invalid` | The secret is dropped. The next `Hello` is a new anonymous slug. Same backoff. |
| `rate_limited` | Wait `retry_after_secs`, or 1 second if that field is missing. The backoff counter resets. |
| `maintenance` | Wait 1 second and dial again. |
| `unauthorized`, `plan_limit` | Process exits 2. Stderr is `tunnel rejected (<code>): <message>`. |
| `version_unsupported`, `name_taken`, `name_invalid`, `name_reserved` | Process exits 1, same stderr line. |
| Any other terminal `Rejected` | Process exits 4. |
| TCP, TLS, or no `Welcome` within 10 seconds | Same backoff. The relay's own deadline for `Hello` is 5 seconds. |

Ctrl-C closes the WebSocket with close code `1000`, then the local
listener, and the process exits 130. The human banner, before the URL, is
`local server is up. waiting for the tunnel URL…`. After `Welcome` it
prints `tunnel: <url>` and `this URL is anonymous and is released 30 minutes after the CLI disconnects`.

Frame types, close codes, and the constant list are in
[Tunnel protocol](tunnel-protocol.md).

## Paste the URL into a client

Use the banner URL. Keep the bearer header. `inspect --client` shows the
local shape; swap the URL.

Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "petstore": {
      "type": "http",
      "url": "https://SLUG.mcp.fetchhive.com/mcp",
      "headers": { "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}" }
    }
  }
}
```

Claude Code:

```bash
claude mcp add --transport http petstore https://SLUG.mcp.fetchhive.com/mcp \
  --header "Authorization: Bearer $MCP_GATEWAY_TOKEN"
```

Claude Code 2.1.281 connects when `tools/list` includes `ttlMs` `0` and
`cacheScope` `private`. `claude mcp list` reports Connected, and a prompt
that calls `logout_user` receives `User logged out`. That check used a local
build of this tree through the production relay. Released `mcp-gateway`
0.7.0 omits both fields, and Claude Code 2.1.281 then rejects `tools/list`
(`ttlMs` must be a number; `cacheScope` must be `public` or `private`). A
server added to a project `.mcp.json` stays unused until
`.claude/settings.json` sets `enabledMcpjsonServers` or
`enableAllProjectMcpServers`.

Codex (`~/.codex/config.toml`):

```toml
[mcp_servers.petstore]
url = "https://SLUG.mcp.fetchhive.com/mcp"
bearer_token_env_var = "MCP_GATEWAY_TOKEN"
default_tools_approval_mode = "approve"
```

Codex 0.156.1 accepts `auto`, `prompt`, `writes`, or `approve` for
`default_tools_approval_mode`. A non-interactive `codex exec` whose approval
policy is `never` refuses the call with `MCP tool call requires approval, but
approval policy is never` until that field is `approve`.

VS Code (`.vscode/mcp.json`):

```json
{
  "servers": {
    "petstore": {
      "type": "http",
      "url": "https://SLUG.mcp.fetchhive.com/mcp",
      "headers": { "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}" }
    }
  }
}
```

OpenAI Responses API (`require_approval` set so the call is not interactive):

```json
{
  "type": "mcp",
  "server_label": "petstore",
  "server_url": "https://SLUG.mcp.fetchhive.com/mcp",
  "headers": { "Authorization": "Bearer $MCP_GATEWAY_TOKEN" },
  "require_approval": "never"
}
```

Anthropic Messages API. Send header `anthropic-beta: mcp-client-2025-11-20`
(the older `mcp-client-2025-04-04` header is deprecated). `authorization_token`
is the raw bearer token. The tool allow-list lives on an `mcp_toolset` in
`tools`, not on the server object:

```json
{
  "mcp_servers": [
    {
      "type": "url",
      "url": "https://SLUG.mcp.fetchhive.com/mcp",
      "name": "petstore",
      "authorization_token": "$MCP_GATEWAY_TOKEN"
    }
  ],
  "tools": [
    {
      "type": "mcp_toolset",
      "mcp_server_name": "petstore",
      "default_config": { "enabled": false },
      "configs": { "get_inventory": { "enabled": true } }
    }
  ]
}
```

ChatGPT connectors and Claude Desktop custom connectors are OAuth-first.
This release has no OAuth on the tunnel. They are not a supported path
unless the product can send a bearer header or you opt into
`--tunnel-auth public` and accept the warning.

## Hosted and open source

| | Where it lives |
|---|---|
| CLI, `mcp-gateway-tunnel`, `mcp-gateway-tunnel-proto`, this doc, [the protocol](tunnel-protocol.md) | this repository |
| Production relay, wildcard certificate, `*.mcp.fetchhive.com` DNS | Fetch Hive |

The relay terminates TLS at the load balancer, then proxies HTTP to the
tunnel process. Access logs store host, method, status, duration, byte
count, request id, and client IP. They do not store bodies, cookies, or
`Authorization`. The reclaim secret is stored as a SHA-256 digest.

## Compatibility

Checked against production `wss://connect.mcp.fetchhive.com/v1/tunnel` with
the Petstore spec (`https://petstore3.swagger.io/api/v3/openapi.json`).
`tools/call` used `find_pets_by_status`. A 5xx from Petstore is the upstream
API, not the tunnel. The curl, OpenAI, Anthropic, Codex, and Inspector rows
used released `mcp-gateway` 0.7.0. The Claude Code row used a local build of
this tree on the same relay. Released 0.7.0 `tools/list` omits `ttlMs` and
`cacheScope`.

| Client | How | Headers | Result | Notes |
|---|---|---|---|---|
| curl | `initialize`, `tools/list`, `tools/call` `find_pets_by_status`, `GET /mcp`, `DELETE /mcp` | yes | pass, 2026-09-24 | `mcp-gateway` 0.7.0, byte-identical to loopback. `initialize` echoed `2025-11-25` (148 bytes) and `serverInfo.version` `0.7.0`. `tools/list` returned 18 tools (9647 bytes). `tools/call` `find_pets_by_status` returned Petstore JSON (409732 bytes). Responses are `content-type: application/json`. `GET /mcp` is 405 with an empty body and no `Allow` header. `DELETE /mcp` is 405 with `Allow: POST,GET,HEAD`. A POST with no `Authorization` is 401, `WWW-Authenticate: Bearer`, and body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"missing authorization"},"id":null}`. `GET /.well-known/oauth-protected-resource` is 404 with an empty body. |
| OpenAI Responses API | `tools: [{ type: "mcp", server_url, headers }]` | yes | pass, 2026-09-24 | Model `gpt-6-astra`, `require_approval: "never"`, `headers.Authorization` set to `Bearer <token>`, `allowed_tools: ["get_inventory"]`. The API returned `mcp_list_tools` with that one tool, then `mcp_call` `get_inventory`. Petstore answered HTTP 500; the gateway returned that as MCP `isError` text. The connector completed the POST path. |
| ChatGPT connectors | UI | OAuth-first | not supported | No OAuth on the tunnel in this release. `--tunnel-auth public` is the only unauthenticated option, with the warning above. |
| Anthropic Messages API | `mcp_servers` plus `tools: [{ type: "mcp_toolset" }]`, header `anthropic-beta: mcp-client-2025-11-20` | yes | pass, 2026-09-24 | Request model `claude-opus-5`. `authorization_token` is the raw token. The response contained `mcp_tool_use` `get_inventory` and `mcp_tool_result` with the same Petstore HTTP 500 text, then `stop_reason: end_turn`. |
| Claude Code | `claude mcp add --transport http` | yes | pass, local build, 2026-09-24 | Claude Code 2.1.281 against a local build of this tree. `claude mcp list` reported Connected. A prompt called `logout_user` and the tool result was `User logged out`. That build's `tools/list` includes `ttlMs` `0` and `cacheScope` `private`. Released `mcp-gateway` 0.7.0 omits both fields, and Claude Code 2.1.281 rejects that `tools/list` (`ttlMs` must be a number; `cacheScope` must be `public` or `private`). A project `.mcp.json` server stays unused until `.claude/settings.json` sets `enabledMcpjsonServers` or `enableAllProjectMcpServers`; until then `claude mcp list` says `Pending approval`. |
| Claude Desktop / claude.ai | UI | OAuth-first | not supported | Same caveat as ChatGPT. |
| Cursor | `.cursor/mcp.json` | yes | not run | A 401 without a header may probe `/.well-known/oauth-protected-resource`. That path is a 404 with an empty body. Send the bearer header. |
| Codex | `bearer_token_env_var` and `default_tools_approval_mode = "approve"` | yes | pass, 2026-09-24 | Codex CLI 0.156.1 called `logout_user` and received `User logged out`. |
| VS Code | `.vscode/mcp.json` `type: http` | yes | not run | VS Code 1.136.0 is installed. `code chat` opens a window and does not return a tool result to the shell, so this row has no recorded call. |
| MCP Inspector | `npx @modelcontextprotocol/inspector --cli` 2.8.0, `--transport http` | yes | pass, 2026-09-24 | `initialize` returned protocol `2025-11-25` and server `0.7.0`. `tools/list` returned 18 tools. `tools/call` `logout_user` returned `User logged out`. `get_inventory` is `isError` with `structuredContent.error_code` `"upstream_5xx"` while `outputSchema` says the values are integers; Inspector 2.8.0 then exits with `data/error_code must be integer`. `get_pet_by_id` returns the tool result and then exits `tool_is_error` because `isError` is true. |
| ngrok or cloudflared in front of `serve` | generic TCP/HTTP tunnel | n/a | not a supported path | See below. |
| Fetch Hive Studio | attach as a workspace MCP server | yes | not run | Paste the banner URL and the bearer. |

## Why not ngrok or cloudflared?

A generic tunnel in front of `mcp-gateway serve` fails MCP clients for
three reasons that this client avoids.

1. **Host allow-list.** Without `--expose`, the local server accepts
   `Host` values `localhost`, `127.0.0.1`, `[::1]`, and `::1` only. ngrok
   sends its own hostname. The tunnel client rewrites `Host` to
   `127.0.0.1:<port>` before the local router sees the request, and keeps
   the public name in `X-Forwarded-Host`.
2. **Browser interstitial.** ngrok's free tier answers clients that do not
   send `ngrok-skip-browser-warning` with an HTML page. OpenAI and
   Anthropic connectors cannot set that header, so they parse HTML instead
   of a JSON-RPC body.
3. **No MCP errors.** A generic proxy does not know `/mcp`. This relay
   returns JSON-RPC for an offline CLI (`503`, code `-32001`,
   `Retry-After: 10`), `401` with `WWW-Authenticate: Bearer` when the token
   is missing, and `429` when the in-flight cap is hit. It does not
   advertise an OAuth discovery URL.

## Troubleshooting

| What you see | What it means |
|---|---|
| `503` and JSON-RPC code `-32001`, `Retry-After: 10` | The lease exists and the CLI socket is down. `GET /health` on that host is `200` with `"online":false`. |
| `401`, `WWW-Authenticate: Bearer`, JSON-RPC `-32000` `missing authorization` | Token mode, and the request had no `Authorization: Bearer` header. |
| `429`, JSON-RPC `-32000` `too many in-flight requests`, `Retry-After: 1` | The CLI already has `max_inflight` calls running (16 unless `Welcome` said otherwise). The local API was not called. |
| `429` `rate limit exceeded` | The relay's per-slug per-minute cap was hit (60 by default). |
| `404` | No lease for that slug. The banner URL is the one that works. |
| Banner says `reconnecting tunnel…` | The CLI is dialing again. The slug stays when the secret is still valid. `reclaim_expired` prints the same line and then a new `tunnel:` URL. |
| New slug after a restart | The reclaim secret is memory-only. A new process does not have it. |
| `tunnel rejected (maintenance): could not allocate a tunnel name` | Eight slug draws were already leased. The CLI waits 1 second and tries again. |
| `tunnel rejected (rate_limited): too many anonymous tunnels from this network` | This IP opened 10 anonymous tunnels in the current hour. The CLI waits `retry_after_secs` (3600 on the production relay). |
| `persistent names are not available yet` | `--name` is hidden until a later release. Drop the flag. |
| `--tunnel` with `--stdio` | Usage error. The tunnel needs the HTTP transport. |
