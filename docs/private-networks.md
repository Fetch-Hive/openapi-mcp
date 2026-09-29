# `--allow-private-networks`

This flag (or `ssrf.allow_private_networks = true`) is a **self-host opt-in**
so you can proxy a local or private API: `localhost`, `127.0.0.1`, RFC1918,
ULA. The process uses the system DNS resolver instead of public resolvers.

That is how you connect a **WIP or branch API** to Cursor / Codex / Claude
Code. Public-internet defaults stay on unless you pass the flag.

Local HTTP APIs also need `--insecure-http` (and `--base-url` when the spec
`servers` entry is relative or points at production).

The same pair of flags allows `add-spec --url` to fetch the OpenAPI document
itself over HTTP. HTTPS stays the default. `http://` is fetched only when
both flags are set and the host is loopback, RFC1918, or IPv6 ULA. A public
`http://` host is refused. Redirects are not followed. The cap is 10 MiB and
the timeout is 15 seconds. Missing either flag exits 2 and names that flag.

```bash
mcp-gateway add-spec --name issues \
  --url http://127.0.0.1:8000/openapi.json \
  --insecure-http \
  --allow-private-networks
```

`serve` uses `--allow-insecure-http` for the same HTTP opt-in. Config keys
`ssrf.allow_insecure_http` and `ssrf.allow_private_networks` count as the
flags when they are `true`.

Cloud metadata addresses stay denied unless the hidden `--allow-metadata`
flag is also set. That hidden flag does not make an `http://` metadata URL
legal for `add-spec`; link-local and the metadata literals stay refused.

This turns the process into a credential-injecting proxy on **your** network.
