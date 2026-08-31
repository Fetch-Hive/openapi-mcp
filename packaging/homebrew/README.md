# Homebrew tap formula template

Live formula: [Fetch-Hive/homebrew-tap](https://github.com/Fetch-Hive/homebrew-tap)
`Formula/mcp-gateway.rb`. cargo-dist's `publish-homebrew-formula` job rewrites
SHA-256s on each `v*` tag.

Users should install with the fully qualified name (Homebrew 6 tap trust):

```bash
brew install Fetch-Hive/tap/mcp-gateway
```

That taps the repo and trusts only this formula. Short name
(`brew install mcp-gateway`) after `brew tap Fetch-Hive/tap` is refused until
`brew trust --formula fetch-hive/tap/mcp-gateway` or `brew trust fetch-hive/tap`.

This `mcp-gateway.rb` is a fallback template (placeholder SHA-256s). Do not copy
zeros into the public tap.
