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
