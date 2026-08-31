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
| Linux | `$XDG_CONFIG_HOME/mcp-gateway/config.toml` | `$XDG_CACHE_HOME/mcp-gateway/ir` |
| macOS | `~/Library/Application Support/mcp-gateway/config.toml` | `~/Library/Caches/mcp-gateway/ir` |
| Windows | `%APPDATA%\mcp-gateway\config.toml` | `%LOCALAPPDATA%\mcp-gateway\ir` |

`$MCP_GATEWAY_CONFIG` and `--config` override the config path.

PaaS (`serve` on Heroku / Render / DigitalOcean): if `--bind` is omitted and
`PORT` is set, the process listens on `0.0.0.0:$PORT` with `--expose` (the
image is distroless, so `$PORT` cannot be interpolated in `CMD`). If `NAME` is
not in config, `serve --url` or `$MCP_GATEWAY_SPEC_URL` compiles that HTTPS
OpenAPI document first. Bearer tokens stay in `$MCP_GATEWAY_TOKEN`, never in
TOML. See [deploy](deploy/README.md).

`[[specs]].url` is the OpenAPI document URL. Relative `servers` entries are
resolved against it. `[[specs]].base_url` (optional) is an absolute upstream
origin that overrides `servers` for `test` and `serve`. For a local checkout,
that is often `http://127.0.0.1:3000` together with
`ssrf.allow_private_networks = true` and `ssrf.allow_insecure_http = true`.
