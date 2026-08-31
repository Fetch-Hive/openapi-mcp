# Deploy mcp-gateway

Self-host the **open-source CLI** (`mcp-gateway serve`) as a long-running HTTP
process. This is not Fetch Hive Cloud.

The published image is `ghcr.io/fetch-hive/mcp-gateway` (**amd64 / x86_64
musl** only today). It is distroless: there is no shell, so platforms must not
rely on `$PORT` expansion in `CMD`. The binary reads `PORT` itself.

## What you set

| Variable | Required | Notes |
|---|---|---|
| `MCP_GATEWAY_TOKEN` | yes | MCP bearer for clients. Never put it in TOML. |
| `MCP_GATEWAY_SPEC_URL` | yes on first boot | HTTPS URL of an OpenAPI document. Same SSRF policy as `add-spec --url`. |
| `PORT` | injected by PaaS | When `--bind` is omitted, listen on `0.0.0.0:$PORT` with `--expose`. |
| `MCP_GATEWAY_CONFIG` | optional | Persist config (Render disk / Hetzner volume): `/data/config.toml`. |

TLS terminates at the platform or your reverse proxy. Binding all interfaces
is `--expose` (automatic when `PORT` is used).

Ephemeral filesystems (Heroku, DigitalOcean App Platform) recompile the spec
from `MCP_GATEWAY_SPEC_URL` on every start.

## Platforms

| Guide | One-click | Persistent disk |
|---|---|---|
| [Render](render.md) | Yes | Yes (paid disk) |
| [Heroku](heroku.md) | Yes (Cedar) | No |
| [DigitalOcean App Platform](digitalocean.md) | Yes | No |
| [Hetzner](hetzner.md) | No (VPS + Compose) | Yes (Volume) |
| [Vercel](vercel.md) | No | n/a — not supported |

Prefer [hosted MCP Gateway](https://fetchhive.com/mcp) if you do not want to
operate a process.
