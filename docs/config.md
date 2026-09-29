# Config schema

See `config.toml.example`. `schema_version = 1`. Unknown keys are errors.

Secrets are references only:

```toml
token = { env = "MCP_GATEWAY_TOKEN" }
# token = { file = "/run/secrets/mcp" }
```

`${VAR}` interpolation is also accepted in string positions. Inline values
that look like `fh_mcp_`, `sk_live`, or `Bearer ` fail `doctor` and `init`.

Platform paths:

| Platform | Config | IR cache |
|---|---|---|
| Linux | `$XDG_CONFIG_HOME/mcp-gateway/config.toml`, or `~/.config/mcp-gateway/config.toml` when that variable is unset | `$XDG_CACHE_HOME/mcp-gateway/ir`, or `~/.cache/mcp-gateway/ir` |
| macOS | `~/Library/Application Support/com.fetchhive.mcp-gateway/config.toml` | `~/Library/Caches/com.fetchhive.mcp-gateway/ir` |
| Windows | `%APPDATA%\fetchhive\mcp-gateway\config\config.toml` | `%LOCALAPPDATA%\fetchhive\mcp-gateway\cache\ir` |

The log file sits next to local data: `~/.local/share/mcp-gateway/mcp-gateway.jsonl`
on Linux (`$XDG_DATA_HOME` when set),
`~/Library/Application Support/com.fetchhive.mcp-gateway/mcp-gateway.jsonl`
on macOS, and `%LOCALAPPDATA%\fetchhive\mcp-gateway\data\mcp-gateway.jsonl`
on Windows. `ProjectDirs` is called as `("com", "fetchhive", "mcp-gateway")`.
Linux uses the application name only. macOS joins all three with dots.
Windows uses `fetchhive\mcp-gateway`.

`$MCP_GATEWAY_CONFIG` and `--config` override the config path.

`[tunnel] relay_url` is optional. `serve --tunnel` uses
`$MCP_GATEWAY_RELAY_URL` when that is set, then this field, then
`wss://connect.mcp.fetchhive.com/v1/tunnel`. See [Tunnel](tunnel.md).

PaaS (`serve` on Heroku / Render / DigitalOcean): if `--bind` is omitted and
`PORT` is set, the process listens on `0.0.0.0:$PORT` with `--expose` (the
image is distroless, so `$PORT` cannot be interpolated in `CMD`). If `NAME` is
not in config, `serve --url` or `$MCP_GATEWAY_SPEC_URL` compiles that
OpenAPI document first. The URL is HTTPS unless it is a loopback, RFC1918,
or ULA `http://` URL and both `ssrf` opt-ins below are set. Bearer tokens stay in `$MCP_GATEWAY_TOKEN`, never in
TOML. See [deploy](deploy/README.md).

`[[specs]].url` is the OpenAPI document URL. Relative `servers` entries are
resolved against it. An `http://` value is stored only when the fetch was
allowed: `ssrf.allow_insecure_http = true` and
`ssrf.allow_private_networks = true`, and the host is loopback, RFC1918, or
IPv6 ULA. Those two keys are the config form of `--insecure-http` and
`--allow-private-networks`. A public `http://` URL is refused (exit 2) even
when both are `true`. Cloud metadata addresses are refused even when both are
`true`. `[[specs]].base_url` (optional) is an absolute upstream origin that
overrides `servers` for `test` and `serve`. For a local checkout, that is
often `http://127.0.0.1:3000` together with the same two `ssrf` keys.
