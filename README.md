# mcp-gateway

_An open-source project by [Fetch Hive](https://fetchhive.com)._

Turn an OpenAPI document into a local MCP server.

`mcp-gateway` compiles OpenAPI 3.0/3.1 to a versioned IR, serves tools over
Streamable HTTP or stdio, and injects upstream credentials from env or file
references. Outbound HTTP uses an SSRF-hardened dialer.

Prefer a hosted MCP Gateway with tokens, quotas, and a dashboard?
https://fetchhive.com/mcp

## Deploy

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/Fetch-Hive/openapi-mcp)
[![Deploy to Heroku](https://www.herokucdn.com/deploy/button.svg)](https://www.heroku.com/deploy?template=https://github.com/Fetch-Hive/openapi-mcp)
[![Deploy to DigitalOcean](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/Fetch-Hive/openapi-mcp/tree/main)

Guides: [Render](docs/deploy/render.md) · [Heroku](docs/deploy/heroku.md) ·
[DigitalOcean](docs/deploy/digitalocean.md) · [Hetzner](docs/deploy/hetzner.md).
Vercel cannot run this server ([why](docs/deploy/vercel.md)).

Set `MCP_GATEWAY_TOKEN` and `MCP_GATEWAY_SPEC_URL` (HTTPS OpenAPI). The image
reads `PORT` itself (distroless, no shell).

## Install

Order: Homebrew → Docker → npx → curl|sh → cargo.

```bash
# Homebrew — use the fully qualified name (Homebrew 6 trusts only this formula).
# `brew tap` then `brew install mcp-gateway` is refused until you `brew trust`.
brew install Fetch-Hive/tap/mcp-gateway

# Docker
docker run --rm -p 127.0.0.1:8787:8787 ghcr.io/fetch-hive/mcp-gateway:0.4.0 version

# npx (optionalDependencies, no postinstall)
npx --yes @fetch-hive/mcp-gateway version

# curl | sh (after the first GitHub Release)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Fetch-Hive/openapi-mcp/releases/latest/download/mcp-gateway-installer.sh | sh

# From source
cargo install --path crates/mcp-gateway-cli
```

## Five-minute quickstart

```bash
mcp-gateway init
mcp-gateway add-spec --name demo --file ./openapi.yaml
# or: mcp-gateway add-spec --name demo --url https://petstore3.swagger.io/api/v3/openapi.json
# Relative OpenAPI `servers` URLs are resolved against the document URL.
# Local files with a relative server need: --base-url https://host/api
export MCP_GATEWAY_TOKEN=…   # printed once by init
mcp-gateway inspect demo     # lists compiled tool names (snake_case from operationId)
# Public Petstore's getInventory often 500s (their demo, not mcp-gateway).
mcp-gateway test demo login_user --args '{"username":"user1","password":"pass"}'
mcp-gateway serve demo
mcp-gateway doctor
```

Point Cursor at `http://127.0.0.1:8787/mcp` with the bearer token, or:

```bash
mcp-gateway inspect demo --client cursor
mcp-gateway serve demo --stdio
```

## Security

Default bind is loopback. Binding `0.0.0.0` requires `--expose`. Upstream
private networks require `--allow-private-networks` (loud; never enabled in
Fetch Hive Cloud). Report vulnerabilities to security@fetchhive.com — see
`SECURITY.md`.

## Docs

- [CLI reference](docs/cli.md)
- [Config schema](docs/config.md)
- [Deploy (PaaS + VPS)](docs/deploy/README.md)
- [SSRF policy](docs/ssrf.md)
- [Private-network flag](docs/private-networks.md)
- [Client snippets](docs/clients.md)

## Licence

Apache License 2.0. See `LICENSE` and `NOTICE`.
