# Render

Best PaaS fit: always-on Docker, a Deploy button, and an optional persistent
disk for config + IR cache.

## One-click

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/Fetch-Hive/openapi-mcp)

The button uses [`render.yaml`](../../render.yaml) in this repo (`repo=` must
be explicit). Auto-deploy from upstream is off so a fork does not redeploy on
every `openapi-mcp` commit.

Set:

- `MCP_GATEWAY_TOKEN` — generate in the Render UI, then put the same value in
  your MCP client.
- `MCP_GATEWAY_SPEC_URL` — HTTPS OpenAPI document.

`PORT` is injected (default **10000**). The start command is exec-form
`serve demo` (the image `ENTRYPOINT` is `mcp-gateway`; no `$PORT` in `CMD`).

## Disk

The Blueprint attaches a disk at `/data` and sets `HOME=/data` plus
`MCP_GATEWAY_CONFIG=/data/config.toml` so IR cache and config survive deploys.
A disk means a **single instance** and no zero-downtime deploys.

Without a disk, every deploy recompiles from `MCP_GATEWAY_SPEC_URL`.

## After deploy

Point Cursor at `https://<service>.onrender.com/mcp` with
`Authorization: Bearer <MCP_GATEWAY_TOKEN>`.
