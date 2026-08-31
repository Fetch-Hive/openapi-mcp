# `--allow-private-networks`

This flag (or `ssrf.allow_private_networks = true`) is a **self-host opt-in**.
It uses the system DNS resolver and skips RFC1918 / ULA denials so you can
proxy to an internal API.

Cloud metadata addresses stay denied unless the hidden `--allow-metadata`
flag is also set.

This turns the process into a credential-injecting proxy on **your** network.
Fetch Hive Cloud never enables this flag. The opt-in is compiled behind the
`self-host` Cargo feature so a hosted binary cannot turn it on.
