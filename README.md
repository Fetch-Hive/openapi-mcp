# mcp-gateway

_An open-source project by [Fetch Hive](https://fetchhive.com)._

**OpenAPI to MCP.** Connect your API to Cursor, Codex, Claude Code, VS Code,
ChatGPT, or any MCP client — in minutes.

That means a **live** API, and it also means the API on your laptop: a WIP
branch, `localhost:3000`, a Docker compose stack. Point the editor at the
gateway and call the same tools you will ship.

Prefer a hosted MCP Gateway with tokens, quotas, and a dashboard?
https://fetchhive.com/mcp

## Two equally important flows

### 1. Local / branch API → editor

Your server is already running on this machine. Compile the OpenAPI, proxy
loopback, paste a snippet into Cursor / Codex / Claude Code.

```bash
mcp-gateway init --allow-private-networks
mcp-gateway add-spec --name demo --file ./openapi.yaml \
  --base-url http://127.0.0.1:3000 --insecure-http
export MCP_GATEWAY_TOKEN=…   # printed once by init
mcp-gateway test demo list_pets --args '{}'
mcp-gateway serve demo
mcp-gateway inspect demo --client cursor      # or: codex | claude-code | vscode | claude
```

`--allow-private-networks` is required for loopback and RFC1918. `--insecure-http`
is required when the API speaks HTTP (typical on localhost). `--base-url` is the
origin of **this** checkout, not production.

### 2. Live API → editor or agent

Same compiler, public HTTPS OpenAPI, no private-network flag.

```bash
mcp-gateway init
mcp-gateway add-spec --name demo --url https://api.example.com/openapi.json
export MCP_GATEWAY_TOKEN=…
mcp-gateway serve demo
mcp-gateway inspect demo --client cursor
```

Or skip operating a process: [hosted MCP Gateway](https://fetchhive.com/mcp).
PaaS / VPS: [deploy](docs/deploy/README.md).

## Deploy

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/Fetch-Hive/openapi-mcp)
[![Deploy to Heroku](https://www.herokucdn.com/deploy/button.svg)](https://www.heroku.com/deploy?template=https://github.com/Fetch-Hive/openapi-mcp)
[![Deploy to DigitalOcean](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/Fetch-Hive/openapi-mcp/tree/main)

Guides: [Render](docs/deploy/render.md) · [Heroku](docs/deploy/heroku.md) ·
[DigitalOcean](docs/deploy/digitalocean.md) · [Hetzner](docs/deploy/hetzner.md).
Vercel cannot run this server ([why](docs/deploy/vercel.md)).

Set `MCP_GATEWAY_TOKEN` and `MCP_GATEWAY_SPEC_URL` (HTTPS OpenAPI). The image
reads `PORT` itself (distroless, no shell). After deploy, paste
`mcp-gateway inspect demo --client cursor` (replace the URL with your
`https://…/mcp`) into the editor.

## Install

Order: Homebrew → Docker → npx → curl|sh → cargo.

```bash
# Homebrew — use the fully qualified name (Homebrew 6 trusts only this formula).
# `brew tap` then `brew install mcp-gateway` is refused until you `brew trust`.
brew install Fetch-Hive/tap/mcp-gateway

# Docker
docker run --rm -p 127.0.0.1:8787:8787 ghcr.io/fetch-hive/mcp-gateway:0.5.0 version

# npx (optionalDependencies, no postinstall)
npx --yes @fetch-hive/mcp-gateway version

# curl | sh (after the first GitHub Release)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Fetch-Hive/openapi-mcp/releases/latest/download/mcp-gateway-installer.sh | sh

# From source
cargo install --path crates/mcp-gateway-cli
```

`mcp-gateway` compiles OpenAPI 3.0/3.1 to a versioned IR, serves tools over
Streamable HTTP or stdio, and injects upstream credentials from env or file
references. Outbound HTTP uses an SSRF-hardened dialer.

## Security

Default bind is loopback. Binding `0.0.0.0` requires `--expose`. Upstream
private networks and localhost require `--allow-private-networks`. Report
vulnerabilities to security@fetchhive.com — see `SECURITY.md`.

## Docs

- [Connect Cursor, Codex, Claude Code](docs/clients.md)
- [CLI reference](docs/cli.md)
- [Config schema](docs/config.md)
- [Deploy (PaaS + VPS)](docs/deploy/README.md)
- [SSRF policy](docs/ssrf.md)
- [Private-network flag](docs/private-networks.md)

## Licence

Apache License 2.0. See `LICENSE` and `NOTICE`.
