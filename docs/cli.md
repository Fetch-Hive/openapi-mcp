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
mcp-gateway tunnel (<URL> | --stdio -- CMD...) [--tunnel-auth token|passthrough|public] [--token TOKEN] [--bind ADDR] [--no-probe] [--allow-remote-upstream]
mcp-gateway login [--no-browser] [--api-url URL] [--force]
mcp-gateway logout [--keep-remote] [--api-url URL]
mcp-gateway whoami [--clear] [--api-url URL]
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

## Account

Login is optional for an anonymous tunnel. It writes an account token that
`serve --name`, `tunnel --name`, and `tunnels` read. `doctor` reads the
file only to check its mode and the stored email. Anonymous tunnels do not
require an account.

```text
mcp-gateway login
mcp-gateway whoami
mcp-gateway logout
```

`login` posts `{client:{name,version,os,hostname}}` to
`POST /v1/anonymous/cli_device/code`. The default API origin is
`https://api.fetchhive.com`. `--api-url` wins over `MCP_GATEWAY_API_URL`,
which wins over `api_url` already stored in `credentials.toml`. The origin
must be `https`, or `http` on `localhost` / `127.0.0.1` / `::1`. A username
or password in the URL is refused. TLS uses rustls with the Mozilla roots
bundled in the binary and the operating system's certificate store.
Certificate checks stay on. A development CA is accepted when the OS trusts
it. A transport failure includes the underlying TLS or connection error.

The command prints `verification_uri_complete` and `user_code`. It opens
that URL in a browser only when the URL is `http` or `https`, stdout is a
terminal, `--json` is absent, and `--no-browser` is absent. Any other
scheme is printed and left closed. It then posts `{device_code}` to
`POST /v1/anonymous/cli_device/token`. The first poll is immediate. Later
polls wait `interval` seconds (`interval` comes from the code response;
production sends `5`). `slow_down` waits `interval + 5` seconds.
`authorization_pending` keeps waiting until `expires_in` seconds have
passed (production sends `900`; `0` is treated as `1` so the first poll
still runs) or the poll returns `expired_token`.
`access_denied` exits 1 with `login denied in browser`. `expired_token`
exits 1 with `code expired; run login again`. Each HTTP call times out
after 15 seconds.

Approval returns `access_token`, `token_type` (`bearer`), `account`
(`id`, `name`, `plan_type`), and `user.email`. `name` is null. The CLI
prints the email and the plan. If `GET /v1/cli/me` fails after the file is
written, tunnel endpoints print `unavailable`. It does not print
`access_token`, `device_code`, or the raw token on success, on failure, or
with `--json`. An error body that contains `fh_cli_` is replaced with
`Fetch Hive returned a token on an error response`.

`credentials.toml` is written beside `config.toml`:

- `--config PATH` and `MCP_GATEWAY_CONFIG`: same directory as that file
- otherwise macOS `~/Library/Application Support/com.fetchhive.mcp-gateway/credentials.toml`
- Linux `$XDG_CONFIG_HOME/mcp-gateway/credentials.toml` (or `~/.config/mcp-gateway/credentials.toml`)
- Windows `%APPDATA%\fetchhive\mcp-gateway\config\credentials.toml`

On Unix the file mode is `0600`. `mcp-gateway doctor` warns when the mode
has group or world bits set. A file that exists but cannot be parsed warns
with the parse error, and is not reported as logged in. It reads the file
only. It does not call the API for this check. Missing credentials are
`not logged in`, not a failure.

```toml
[fetchhive]
api_url = "https://api.fetchhive.com"
token = "fh_cli_…"
account_id = "…"
user_email = "…"
plan_type = "developer"
logged_in_at = "2026-09-26T00:00:00Z"
```

`account_name` is omitted when the API sends null.

Token lookup for `whoami` and `logout` is `--cli-token` (hidden), then
`MCP_GATEWAY_CLI_TOKEN`, then `credentials.toml`. `tunnels`, `serve --name`,
and `tunnel --name` use the env var and then the file. They have no
`--cli-token` flag. A 401 from those commands leaves `credentials.toml` in
place. The env var and the flag
override the file. `logout` does not delete the file and does not call
`DELETE /v1/cli/token` when the token came from the env var or `--cli-token`.
Unset the variable first. A file token calls `DELETE /v1/cli/token` and then
deletes the file. HTTP 204, 401, and 404 all delete the file. Any other
HTTP or network error leaves the file in place. `--keep-remote` deletes the
file and skips the DELETE.

`whoami` calls `GET /v1/cli/me`. No token prints
`Not logged in. Run mcp-gateway login.` and exits 0. `--json` prints
`{"logged_in":false}`. A 401 prints
`Token revoked or expired; run mcp-gateway login` and exits 1.
`--clear` deletes `credentials.toml` on that 401 when the token came from
that file. A token from `MCP_GATEWAY_CLI_TOKEN` or `--cli-token` leaves the
file in place. A live token prints the email, plan, token name, `last_four`,
and tunnel endpoint `used/limit`. `limit` null prints `N (no cap)`.

`login` with a credentials file already present prints that email, plan, and
`logged_in_at`, then exits 0. `MCP_GATEWAY_CLI_TOKEN` alone prints that the
variable is set. It does not print an email, because the variable is not
the file. `--force` starts the device flow again and overwrites the file.
`--json` prints one object per event:
`{"event":"device_code","user_code","verification_uri_complete"}` then
`{"event":"logged_in",...}`. An existing login prints
`{"event":"already_logged_in","source","user","account","logged_in_at"}`.
None of those objects contain `device_code` or `access_token`. `--quiet`
does not hide those lines.

HTTP 503 `cli_device_unavailable` exits 4:
`Fetch Hive login is temporarily unavailable; local commands and anonymous tunnels still work.`
HTTP 429 exits 4 and includes `Retry-After` when the header is a number of
seconds. A transport failure exits 1 and names `MCP_GATEWAY_API_URL`.

`MCP_GATEWAY_CLI_TOKEN=… mcp-gateway whoami` works with no file present.

## Tunnel

`mcp-gateway serve NAME --tunnel` keeps the local server on loopback and opens
an outbound WebSocket to `wss://connect.mcp.fetchhive.com/v1/tunnel` (override
with `MCP_GATEWAY_RELAY_URL` or `[tunnel] relay_url` in config). The status
screen prints `https://<slug>.mcp.fetchhive.com/mcp`. That URL is anonymous
and is released 30 minutes after the CLI disconnects. `--tunnel` cannot be
combined with `--stdio`. `--name SLUG` implies `--tunnel` and reserves that
hostname. See [Persistent names](tunnel.md#persistent-names).

`--tunnel-auth token` (the default) requires `MCP_GATEWAY_TOKEN` or
`--token-file`. Remote clients must send that bearer token.
`--tunnel-auth public` requires `--allow-anonymous`. The auth row then reads
`public, this URL is reachable by anyone on the internet`. `--json` or
`--quiet` prints
`warning: this tunnel URL is reachable by anyone on the internet with no token`
on stderr instead of drawing that row. The screen does not print the
hosted-login line. `--json` emits one compact JSON object per line
(`{"event":"tunnel",...}` and `{"event":"request","method","status","duration_ms"}`)
and does not draw the screen. Ctrl-C exits 130 after the WebSocket closes.

`MCP_GATEWAY_RELAY_URL` overrides the relay. `[tunnel] relay_url` in
config is the fallback. The default is
`wss://connect.mcp.fetchhive.com/v1/tunnel`. An empty token in token mode
exits 1 and names `MCP_GATEWAY_TOKEN`, `--token-file`, or
`--tunnel-auth public`. `--name` with no account token exits 1 before any
socket opens. The sentence is in [Persistent names](tunnel.md#persistent-names).

A terminal `Rejected` exits 2 for `unauthorized` and `plan_limit`, 1 for
`version_unsupported` and `name_taken` / `name_invalid` / `name_reserved`,
and 4 for every other code. Stderr is `tunnel rejected (<code>): <message>`.
`reclaim_expired`, `reclaim_invalid`, `rate_limited`, and `maintenance` do
not exit. They dial again. Ctrl-C exits 130.

The client waits 10 seconds for `Welcome`. Retry delays, the lease key, and
the public status bodies are in [Tunnel](tunnel.md). Framing and close
codes are in [Tunnel protocol](tunnel-protocol.md).

## tunnel

`mcp-gateway tunnel` exposes a Streamable HTTP MCP server, or a stdio MCP
server, through the same anonymous relay. `serve NAME --tunnel` is a
different command: it compiles an OpenAPI spec. `--tunnel-auth` here is
`token` (default), `passthrough`, or `public`. `passthrough` is not accepted
by `serve`.

```bash
mcp-gateway tunnel http://127.0.0.1:8000/mcp
mcp-gateway tunnel --stdio -- npx -y @modelcontextprotocol/server-filesystem /tmp
```

A URL and `--stdio` together exit 1. Neither argument exits 1. `--bind` with
a URL exits 1. The stdio default bind is `127.0.0.1:8787` and the path is
`/mcp`. `--name SLUG` reserves that hostname. See
[Persistent names](tunnel.md#persistent-names).

Loopback, RFC1918, and IPv6 ULA are allowed on any port.
`--allow-remote-upstream` is required for a public address (exit 1 without
it). Metadata and the other outbound denylist ranges stay refused.
`--allow-private-networks` does not change that check. Timeouts, the 502 and
504 bodies, header stripping, id remapping, and the stdio restart cap are in
[Tunnel](tunnel.md#any-mcp-server).

Token mode with an empty `MCP_GATEWAY_TOKEN` draws a 43-character base64url
token and shows it once. `--json` and `--quiet` print
`MCP_GATEWAY_TOKEN=<token>` on stderr and do not put it in the JSON events.
`--no-probe` skips the HTTP `initialize` / `tools/list` check. The stdio
bridge still sends one `initialize`, because later `initialize` calls are
answered from that cache.
