# Heroku

One-click on **Cedar** with the container stack. The dyno filesystem is
**ephemeral**: every restart recompiles from `MCP_GATEWAY_SPEC_URL`. Eco dynos
sleep; use a paid web dyno if clients need the process always up.

## One-click

[![Deploy to Heroku](https://www.herokucdn.com/deploy/button.svg)](https://www.heroku.com/deploy?template=https://github.com/Fetch-Hive/openapi-mcp)

The `template=` query is required. Buttons that omit it fail when the browser
strips `Referer`. Fir-generation apps cannot be created from Buttons.

[`app.json`](../../app.json) declares env; [`heroku.yml`](../../heroku.yml)
builds [`docker/Dockerfile`](../../docker/Dockerfile). `run.web` is `serve demo`
(args to the image `ENTRYPOINT`). Heroku injects `PORT`; do not put `$PORT` in
the start command.

## Env

| Name | Required |
|---|---|
| `MCP_GATEWAY_TOKEN` | yes (button can generate) |
| `MCP_GATEWAY_SPEC_URL` | yes — HTTPS OpenAPI URL |

## After deploy

`https://<app>.herokuapp.com/mcp` with the bearer token. Point Cursor, Codex,
or Claude Code at that URL the same way as local
([clients](../clients.md)).
