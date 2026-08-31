# Homebrew

Install with the fully qualified name (Homebrew 6 tap trust):

```bash
brew install Fetch-Hive/tap/mcp-gateway
```

That taps [Fetch-Hive/homebrew-tap](https://github.com/Fetch-Hive/homebrew-tap)
and trusts only this formula. Short name (`brew install mcp-gateway`) after
`brew tap Fetch-Hive/tap` is refused until
`brew trust --formula fetch-hive/tap/mcp-gateway` or `brew trust fetch-hive/tap`.
