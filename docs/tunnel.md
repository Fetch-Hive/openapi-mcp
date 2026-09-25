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

Tunnels in this release are anonymous. `mcp-gateway login` stores an account
token for a later release. `serve` and `tunnel` do not read it. There is no
`tunnels` command, and `--name` exits with "persistent names are not
available yet". `mcp-gateway tunnel` exposes a Streamable HTTP server or a
stdio server you already run. `serve NAME --tunnel` is unchanged.

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

A dropped socket dials again. On a terminal, the status row changes to
`reconnecting (attempt N, next dial in D)`. `N` is 0 on the first retry.
`D` is the sleep before the next dial: `1.0s` at one second or more, otherwise
a millisecond count such as `800ms`. That sleep is 1s, then 2, 4, 8, 16, 32,
60, each multiplied by a random factor in [0.8, 1.2), unless a frame named an
exact wait.

`--json` does not draw the screen. Each event is one line of compact JSON:

```json
{"event":"tunnel","state":"connecting"}
{"event":"tunnel","state":"connected","url":"https://SLUG.mcp.fetchhive.com/mcp","slug":"SLUG"}
{"event":"tunnel","state":"reconnecting","attempt":0,"delay_ms":1000}
{"event":"request","method":"POST","status":200,"duration_ms":12}
{"event":"tunnel","state":"rejected","code":"unauthorized","message":"…"}
{"event":"tunnel","state":"stopped"}
```

`attempt` matches the status row: 0 is the first retry. `delay_ms` is the
sleep in milliseconds, truncated toward zero. `duration_ms` is the same
truncation of the time from accept until the local call finishes. `status`
`0` means the relay cancelled the call before the local server returned a
status. A `429` from the in-flight cap is logged and is not included in
`total`, because that call was refused before it ran. Every accepted call,
including status `0`, increments `total`.

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
listener, and the process exits 130.

While that process is running, the terminal shows eight header
rows: session status (`connecting`, `online`, or the reconnecting line
above), CLI version, local URL `http://127.0.0.1:<port>/mcp`, remote URL
(or `waiting` until `Welcome`), auth, the lease line
`anonymous, released 30 minutes after disconnect`, a blank row, and
`Requests` with `in-flight`, `total`, and `reconnects`. When stdout is a
terminal at least 11 rows tall and wider than the auth line, those eight
rows stay fixed and each finished call scrolls underneath as
`METHOD STATUS DURATION` (for example `POST    200 12ms`). A pipe, or a
terminal that is too small, prints the eight rows once, then reprints the
status, remote URL, and requests rows on each state change, and appends one
request line plus a requests row after each finished call. `--json` and
`--quiet` print none of that screen. Auth is `bearer required`, or
`public, this URL is reachable by anyone on the internet`. In `--json` or
`--quiet` with `--tunnel-auth public`, stderr still prints
`warning: this tunnel URL is reachable by anyone on the internet with no token`.

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

Cursor 3.21.18 loads this file. `agent mcp list` leaves the server unloaded
until `agent mcp enable petstore`. After that, `agent mcp list-tools petstore`
returns the tool names. A non-interactive `agent -p` run rejects the tool
call with `User rejected MCP: petstore-logout_user` until the same command
includes `--force`, which then receives `User logged out` from `logout_user`.

Claude Code:

```bash
claude mcp add --transport http petstore https://SLUG.mcp.fetchhive.com/mcp \
  --header "Authorization: Bearer $MCP_GATEWAY_TOKEN"
```

Claude Code 2.1.281 connects to `mcp-gateway` 0.7.1. `claude mcp list`
reports Connected, and a prompt that calls `logout_user` receives
`User logged out`. `tools/list` includes `ttlMs` `0` and `cacheScope`
`private`. Released 0.7.0 omits both fields, and Claude Code 2.1.281 then
rejects `tools/list` (`ttlMs` must be a number; `cacheScope` must be
`public` or `private`). A server added to a project `.mcp.json` stays unused
until `.claude/settings.json` sets `enabledMcpjsonServers` or
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

VS Code 1.136.0 with Copilot Chat 0.64.0 loads this file after the folder is
trusted. In Agent mode, a prompt that calls `logout_user` receives
`User logged out`. The chat names the server `petstore (MCP Server)`.

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

## Any MCP server

`mcp-gateway tunnel` is the same anonymous relay client as `serve --tunnel`,
in front of a server this process did not compile.

```bash
mcp-gateway tunnel http://127.0.0.1:8000/mcp
mcp-gateway tunnel --stdio -- npx -y @modelcontextprotocol/server-filesystem /tmp
```

Pass a URL or `--stdio`, not both. `--bind` is only valid with `--stdio`.
The default bind is `127.0.0.1:8787`, path `/mcp`. `--name` exits 1 with
"persistent names are not available yet" before any socket opens.

### Where the process will dial

Startup resolves the URL once with the system resolver. Every address in
that answer must pass the check below. The process then pins the first
address in the list for the life of the process. Later DNS answers are not
used. The client does not read `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY`.

Allowed without a flag: loopback (`127.0.0.0/8`, `::1`), RFC1918
(`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), and IPv6 ULA (`fc00::/7`),
on any port. `serve` uses a different outbound check: loopback is refused
and the only open ports are 80, 443, and 8443. `tunnel` does not call that
check.

A public address is refused until you pass `--allow-remote-upstream`. Exit 1.
The error names the address and the flag. Addresses the outbound denylist
already blocks stay refused with that flag too. Exit 1. The error says the
address is blocked. That includes `169.254.169.254`, `168.63.129.16`,
`100.100.100.200`, and AWS IMDSv6 `fd00:ec2::254`. `fd00:ec2::254` is inside
ULA (`fc00::/7`) and is still refused. Other ULA addresses stay allowed.
Documentation ranges and benchmarking ranges are refused the same way. A scheme other than `http` or `https`, a
URL with a username or password, or port 0 is the same exit. DNS failure is
exit 4: `could not resolve <host>: <error>`.

`--allow-private-networks` does not change this check.

### HTTP upstream

The client is reqwest on rustls. Connect budget is 5 seconds. The whole
request, including a `text/event-stream` body, ends at 60 seconds. Idle
pooled connections are dropped after 90 seconds. Redirects are not followed:
a 301 or 302 is returned with that status.

The relay path `/mcp` is replaced by the URL you passed, including its query.
The incoming query is dropped. `Host` is the upstream authority. Hop-by-hop
headers (`Connection`, `Keep-Alive`, `TE`, `Trailer`, `Transfer-Encoding`,
`Upgrade`, and any `Proxy-*`), `Cookie`, and `Content-Length` are dropped.
`Accept`, `Content-Type`, `Mcp-Session-Id`, `MCP-Protocol-Version`,
`Last-Event-ID`, and `Authorization` are copied when they are still present.
`User-Agent` is `mcp-gateway/<version> (<os>-<arch>)` unless the request
already has one.

A connection failure is HTTP 502. A timeout is HTTP 504. Both bodies are

```json
{"jsonrpc":"2.0","error":{"code":-32001,"message":"upstream unreachable"},"id":null}
```

with `upstream timeout` in the 504 body. Stderr also prints `upstream: ` and
the client error. HTTPS uses the webpki root set. A private CA fails the
handshake and becomes that 502.

Before the tunnel opens, the CLI `POST`s `initialize` with
`protocolVersion` `2025-06-18`, then `notifications/initialized`, then
`tools/list`. `Accept` is `application/json, text/event-stream`. A
`text/event-stream` body is parsed from its `data:` lines. Failure exits 4
and prints whether the server is running and whether the path is Streamable
HTTP. The older SSE transport (`GET /sse` and `POST /messages`) is not
proxied. `--no-probe` skips those three calls. The Probe row then says
`skipped`.

### Auth

`--tunnel-auth token` is the default. If `--token` and `MCP_GATEWAY_TOKEN`
are both empty, the process draws 32 bytes from the OS CSPRNG and encodes
them as base64url with no padding (43 characters). That value stays in
memory. It is not written to config and it is not a field on `--json`
events. A terminal shows it in the Auth row as `bearer <token>` for the
session. `--json` and `--quiet` print `MCP_GATEWAY_TOKEN=<token>` once on
stderr. A token you passed is not printed again. Prefer the environment
variable: `--token` puts the secret in the process arguments.

Token mode compares `Authorization: Bearer` in constant time when the
lengths match. A missing header, a value that is not UTF-8, or a scheme
other than the exact prefix `Bearer ` is HTTP 401, `WWW-Authenticate: Bearer`,
and `missing authorization`. A different length, or the same length that
does not match, is `invalid authorization`. The header is removed before
the upstream or the child sees the request. On the wire the relay is in
`token` mode, so a missing `Authorization` is rejected by the relay and
never reaches this process.

`--tunnel-auth passthrough` does not check and does not remove
`Authorization`. The relay is told `public`, because relay `token` mode
answers 401 itself when the header is missing and the upstream would never
see that request. The Auth row reads
`passthrough, upstream sees the caller's Authorization`. The probe sends
`Authorization: Bearer` only in this mode, and only when `--token` or
`MCP_GATEWAY_TOKEN` is set.

`--tunnel-auth public` does not check and does remove `Authorization`. The
relay is `public`. The Auth row uses the same sentence as `serve --tunnel`.
`--json` or `--quiet` prints
`warning: this tunnel URL is reachable by anyone on the internet with no token`
on stderr.

### stdio

`--stdio` takes everything after `--` as the child command.
`stdin` and `stdout` are newline-delimited JSON-RPC. The child's stderr is
copied here with the prefix `stdio: `. The child inherits this process's
environment. `MCP_GATEWAY_TOKEN` is removed before the child is spawned.
The first stdout line that is not
JSON prints
`warning: stdio child wrote a non-JSON line; further non-JSON lines are discarded`.

Startup sends one `initialize` (`protocolVersion` `2025-06-18`,
`capabilities` `{}`, `clientInfo.name` `mcp-gateway`, `clientInfo.version`
this build) and one `notifications/initialized`. Each of those calls, and
the startup `tools/list`, waits 60 seconds. If the child does not answer,
the CLI exits 4 with `<method> timed out after 60 seconds` and does not
start the child again. That `InitializeResult` is
cached. Every later `initialize` returns the cache with the caller's `id`
and is not written to the child. The child's protocol version is whatever
it returned; it is not negotiated again. A remote
`notifications/initialized` is HTTP 202 with an empty body and is not
written to the child. Other remote notifications are written through and
answered with HTTP 202.

Other requests replace `id` with a monotonic `u64` so two clients can reuse
an id. The response puts the caller's `id` back. The wait is 60 seconds,
then HTTP 504 `upstream timeout`. That id is dropped, so a late child
response for it is discarded. A body over 1048576 bytes is HTTP 413
`body exceeds 1048576 bytes`. A JSON array is HTTP 400
`JSON-RPC batches are not accepted`. `GET /mcp` is HTTP 405 with an empty
body. `DELETE /mcp` is HTTP 200 with an empty body. Neither is written to
the child.

A stdout line with both `method` and `id` is a request from the child
(`sampling/createMessage`, `roots/list`, elicitation, and anything else).
The bridge writes back JSON-RPC `-32601` and
`server requests are not bridged`, and prints
`warning: child request <method> is not bridged; answered with JSON-RPC -32601`.
A child notification (no `id`, including `notifications/tools/list_changed`)
is discarded. The first time each method is seen, stderr prints
`warning: child notification <method> was discarded`. Remote clients are
not told.

If the child exits, in-flight HTTP calls get 502 `upstream unreachable`.
The process is started again after 200ms, then 400ms, 800ms, 1600ms, and
3200ms. The cached initialize result is replaced by the new child's result.
A sixth exit inside the same 60 seconds stops the CLI with exit 4. A child
that answers `initialize` or `tools/list` with a JSON-RPC error also exits 4
and does not retry. A command that cannot be spawned exits 4 immediately.
`--no-probe` still sends the startup `initialize` (the cache needs it) and
skips the startup `tools/list`. The Probe row then ends with
`tools/list skipped`.

One child, one session. Responses are read in order. There is no second
stdio session and no server-to-client request forwarded to the remote MCP
client.

### Screen

The header is the eight `serve --tunnel` rows plus two: `Upstream` and
`Probe`, inserted between Auth and Lease. A pinned header is 10 rows and
needs a terminal at least 13 rows tall and wider than the longest row. A
generated token lives in the Auth row so the clear does not erase it.
`--json` still prints one compact object per line for `tunnel` and
`request` events, and does not draw the screen.

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
used released `mcp-gateway` 0.7.0. The Claude Code, Cursor, VS Code, and
Fetch Hive Studio rows used released `mcp-gateway` 0.7.1 on the same relay. 0.7.1 `tools/list` includes
`ttlMs` `0` and `cacheScope` `private`. Released 0.7.0 omits both fields.

| Client | How | Headers | Result | Notes |
|---|---|---|---|---|
| curl | `initialize`, `tools/list`, `tools/call` `find_pets_by_status`, `GET /mcp`, `DELETE /mcp` | yes | pass, 2026-09-24 | `mcp-gateway` 0.7.0, byte-identical to loopback. `initialize` echoed `2025-11-25` (148 bytes) and `serverInfo.version` `0.7.0`. `tools/list` returned 18 tools (9647 bytes). `tools/call` `find_pets_by_status` returned Petstore JSON (409732 bytes). Responses are `content-type: application/json`. `GET /mcp` is 405 with an empty body and no `Allow` header. `DELETE /mcp` is 405 with `Allow: POST,GET,HEAD`. A POST with no `Authorization` is 401, `WWW-Authenticate: Bearer`, and body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"missing authorization"},"id":null}`. `GET /.well-known/oauth-protected-resource` is 404 with an empty body. |
| OpenAI Responses API | `tools: [{ type: "mcp", server_url, headers }]` | yes | pass, 2026-09-24 | Model `gpt-6-astra`, `require_approval: "never"`, `headers.Authorization` set to `Bearer <token>`, `allowed_tools: ["get_inventory"]`. The API returned `mcp_list_tools` with that one tool, then `mcp_call` `get_inventory`. Petstore answered HTTP 500; the gateway returned that as MCP `isError` text. The connector completed the POST path. |
| ChatGPT connectors | UI | OAuth-first | not supported | No OAuth on the tunnel in this release. `--tunnel-auth public` is the only unauthenticated option, with the warning above. |
| Anthropic Messages API | `mcp_servers` plus `tools: [{ type: "mcp_toolset" }]`, header `anthropic-beta: mcp-client-2025-11-20` | yes | pass, 2026-09-24 | Request model `claude-opus-5`. `authorization_token` is the raw token. The response contained `mcp_tool_use` `get_inventory` and `mcp_tool_result` with the same Petstore HTTP 500 text, then `stop_reason: end_turn`. |
| Claude Code | `claude mcp add --transport http` | yes | pass, 2026-09-24 | Claude Code 2.1.281 against released `mcp-gateway` 0.7.1. `claude mcp list` reported Connected. A prompt called `logout_user` and the tool result was `User logged out`. `tools/list` includes `ttlMs` `0` and `cacheScope` `private`. Released 0.7.0 omits both fields, and Claude Code 2.1.281 rejects that `tools/list` (`ttlMs` must be a number; `cacheScope` must be `public` or `private`). A project `.mcp.json` server stays unused until `.claude/settings.json` sets `enabledMcpjsonServers` or `enableAllProjectMcpServers`; until then `claude mcp list` says `Pending approval`. |
| Claude Desktop / claude.ai | UI | OAuth-first | not supported | Same caveat as ChatGPT. |
| Cursor | `.cursor/mcp.json` | yes | pass, 2026-09-24 | Cursor 3.21.18 and agent CLI 2026.09.23-86fc751 against released `mcp-gateway` 0.7.1. Project `.cursor/mcp.json` set `headers.Authorization` to `Bearer ${env:MCP_GATEWAY_TOKEN}`. `agent mcp list` showed the server as not loaded until `agent mcp enable petstore`. `agent mcp list-tools petstore` then returned 18 tools, including `logout_user`. `agent -p` without `--force` returned `User rejected MCP: petstore-logout_user`. The same prompt with `--force` called `logout_user` and the tool result was `User logged out`. A 401 without a header may probe `/.well-known/oauth-protected-resource`. That path is a 404 with an empty body. Send the bearer header. |
| Codex | `bearer_token_env_var` and `default_tools_approval_mode = "approve"` | yes | pass, 2026-09-24 | Codex CLI 0.156.1 called `logout_user` and received `User logged out`. |
| VS Code | `.vscode/mcp.json` `type: http` | yes | pass, 2026-09-25 | VS Code 1.136.0 and Copilot Chat 0.64.0 against released `mcp-gateway` 0.7.1. Project `.vscode/mcp.json` sent `Authorization: Bearer`. The folder must be trusted; Restricted Mode does not load the server. Agent mode, model `MAI-Code-1.1-Flash`, called `logout_user` (`petstore (MCP Server)`) and the tool result was `User logged out`. |
| MCP Inspector | `npx @modelcontextprotocol/inspector --cli` 2.8.0, `--transport http` | yes | pass, 2026-09-24 | `initialize` returned protocol `2025-11-25` and server `0.7.0`. `tools/list` returned 18 tools. `tools/call` `logout_user` returned `User logged out`. `get_inventory` is `isError` with `structuredContent.error_code` `"upstream_5xx"` while `outputSchema` says the values are integers; Inspector 2.8.0 then exits with `data/error_code must be integer`. `get_pet_by_id` returns the tool result and then exits `tool_is_error` because `isError` is true. |
| ngrok or cloudflared in front of `serve` | generic TCP/HTTP tunnel | n/a | not a supported path | See below. |
| Fetch Hive Studio | Settings → Connected MCP servers | yes | pass, 2026-09-25 | `app.fetchhive.com` against released `mcp-gateway` 0.7.1. Connected `https://<slug>.mcp.fetchhive.com/mcp` with auth type Access token. Test Connection returned `Connection successful` and 18 tools (`update_pet`, `add_pet`, `find_pets_by_status`, `find_pets_by_tags`, `get_pet_by_id`, and 13 more). The server was saved as `petstore-tunnel-test`, then removed. A Studio agent did not call `logout_user`. |

## Generic proxy

`mcp-gateway tunnel` was checked from this working tree against production
`wss://connect.mcp.fetchhive.com/v1/tunnel`. `mcp-gateway version` on that
binary still prints `0.7.2`. Each process was stopped after the calls, and
the anonymous slug was released.

### Streamable HTTP

FastMCP 4.0.9 on Python 3.14.2. The server name was `weather-server`,
version `1.2.0`, stateless JSON at `http://127.0.0.1:8791/mcp`, one tool
`fh_ping`.

| Client | Result | Notes |
|---|---|---|
| curl | pass, 2026-09-25 | `initialize` HTTP 200, `serverInfo.name` `weather-server`, `serverInfo.version` `1.2.0`. `tools/list` HTTP 200, tool names `fh_ping`. `tools/call` `fh_ping` with `{"name":"ada"}` returned text `pong ada` and `isError` false. |
| OpenAI Responses API | pass, 2026-09-25 | Model `gpt-6-astra`, `require_approval` `never`, `allowed_tools` `["fh_ping"]`, `headers.Authorization` `Bearer <token>`. HTTP 200, status `completed`. Output items were `mcp_list_tools` with `fh_ping`, `mcp_call` `fh_ping` output `pong ada`, then a message `pong ada`. |
| Cursor | pass, 2026-09-25 | Agent CLI 2026.09.23-86fc751. An isolated project `.cursor/mcp.json` used `type` `http` and `Authorization` `Bearer ${env:MCP_GATEWAY_TOKEN}`. `agent -p --trust --approve-mcps` returned `User rejected MCP: fh-weather-fh_ping`. The same prompt with `--force` replied `pong ada`. |

### stdio

```bash
mcp-gateway tunnel --stdio -- npx -y @modelcontextprotocol/server-filesystem <dir>
```

The child reported `secure-filesystem-server` version `0.2.0`. `tools/list`
returned 14 tools: `read_file`, `read_text_file`, `read_media_file`,
`read_multiple_files`, `write_file`, `edit_file`, `create_directory`,
`list_directory`, `list_directory_with_sizes`, `directory_tree`,
`move_file`, `search_files`, `get_file_info`, `list_allowed_directories`.

| Client | Result | Notes |
|---|---|---|
| curl | pass, 2026-09-25 | `initialize` HTTP 200. `tools/call` `read_text_file` on a file whose contents are `stdio-ok` plus a trailing newline returned that text. The result had no `isError` field. |
| OpenAI Responses API | pass, 2026-09-25 | Model `gpt-6-astra`, `require_approval` `never`, `allowed_tools` `["read_text_file"]`. HTTP 200, status `completed`. `mcp_list_tools` listed `read_text_file`. `mcp_call` output was `stdio-ok` plus a trailing newline. The following message was `stdio-ok`. |
| Cursor | pass, 2026-09-25 | Agent CLI 2026.09.23-86fc751 with `--trust --approve-mcps --force` in an isolated project. The reply was `stdio-ok`. |

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
| Status says `reconnecting (attempt N, next dial in D)` | The CLI is waiting `D`, then dialing again. `N` starts at 0. The slug stays when the secret is still valid. `reclaim_expired` uses the same status and then a new remote URL. |
| New slug after a restart | The reclaim secret is memory-only. A new process does not have it. |
| `tunnel rejected (maintenance): could not allocate a tunnel name` | Eight slug draws were already leased. The CLI waits 1 second and tries again. |
| `tunnel rejected (rate_limited): too many anonymous tunnels from this network` | This IP opened 10 anonymous tunnels in the current hour. The CLI waits `retry_after_secs` (3600 on the production relay). |
| `persistent names are not available yet` | `--name` is hidden until a later release. Drop the flag. |
| `--tunnel` with `--stdio` | Usage error. The tunnel needs the HTTP transport. |
