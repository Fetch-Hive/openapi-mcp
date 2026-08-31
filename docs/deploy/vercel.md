# Vercel

**Not supported.** Vercel runs container images as Functions: request-scoped,
duration-capped, scale-to-zero, no attached disk. Streamable HTTP MCP needs a
process that stays up between client requests.

Use [Render](render.md), a [Hetzner VPS](hetzner.md), or
[hosted MCP Gateway](https://fetchhive.com/mcp).

Do not add a Vercel Deploy button for this binary.
