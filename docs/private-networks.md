# `--allow-private-networks`

This flag (or `ssrf.allow_private_networks = true`) is a **self-host opt-in**
so you can proxy a local or private API: `localhost`, `127.0.0.1`, RFC1918,
ULA. The process uses the system DNS resolver instead of public resolvers.

That is how you connect a **WIP or branch API** to Cursor / Codex / Claude
Code. Public-internet defaults stay on unless you pass the flag.

Local HTTP APIs also need `--insecure-http` (and `--base-url` when the spec
`servers` entry is relative or points at production).

Cloud metadata addresses stay denied unless the hidden `--allow-metadata`
flag is also set.

This turns the process into a credential-injecting proxy on **your** network.
