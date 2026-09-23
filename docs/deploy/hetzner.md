# Hetzner Cloud

There is no marketplace one-click for this binary. Create a CX/CPX **amd64**
VPS, install Docker (or use the Docker CE Cloud App), and run Compose. The
image is `ghcr.io/fetch-hive/mcp-gateway` (linux/amd64 and linux/arm64). Put config and IR cache
on a [Volume](https://docs.hetzner.com/cloud/volumes/overview/).

TLS: Caddy or nginx on the host; the container speaks HTTP.

## Compose

See [`docker/compose.yaml`](../../docker/compose.yaml). Set `MCP_GATEWAY_TOKEN`
and either:

- `MCP_GATEWAY_SPEC_URL` and `command: ["serve", "demo"]` with `PORT=8787`, or
- a host `config.toml` as in the file comments.

## Cloud-init

Paste [`docker/cloud-init.yaml`](../../docker/cloud-init.yaml) into the Console
cloud-config field (or API `user_data`) after replacing the token, spec URL,
and image tag. Attach and mount a Volume at `/mnt/data` if you want persistence
across rebuilds.

Put TLS in front, then point Cursor, Codex, or Claude Code at
`https://your-host/mcp` ([clients](../clients.md)).
