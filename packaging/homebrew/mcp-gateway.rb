# Frozen formula template. Copy into Fetch-Hive/homebrew-tap/Formula/mcp-gateway.rb
# after the first GitHub Release exists, then replace URL and sha256.

class McpGateway < Formula
  desc "Turn an OpenAPI document into an MCP server"
  homepage "https://github.com/Fetch-Hive/openapi-mcp"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/Fetch-Hive/openapi-mcp/releases/download/v0.1.0/mcp-gateway-aarch64-apple-darwin.tar.xz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/Fetch-Hive/openapi-mcp/releases/download/v0.1.0/mcp-gateway-x86_64-apple-darwin.tar.xz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Fetch-Hive/openapi-mcp/releases/download/v0.1.0/mcp-gateway-aarch64-unknown-linux-musl.tar.xz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/Fetch-Hive/openapi-mcp/releases/download/v0.1.0/mcp-gateway-x86_64-unknown-linux-musl.tar.xz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "mcp-gateway"
  end

  test do
    system "#{bin}/mcp-gateway", "version"
  end
end
