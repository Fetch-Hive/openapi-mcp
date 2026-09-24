use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "mcp-gateway",
    version,
    about = "OpenAPI to MCP — connect a live or local API to Cursor, Codex, Claude Code, and agents.",
    long_about = None
)]
pub struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Parser)]
pub struct Globals {
    /// Path to config.toml (overrides $MCP_GATEWAY_CONFIG and the platform default).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Increase log verbosity.
    #[arg(short, long, action = ArgAction::Count, global = true)]
    pub verbose: u8,
    /// Suppress non-error output.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Emit JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Colorize output.
    #[arg(long, global = true, default_value = "auto")]
    pub color: ColorMode,
    /// Reach RFC1918, ULA, and loopback (local / branch APIs). Loud opt-in.
    #[arg(long, global = true)]
    pub allow_private_networks: bool,
    /// Also allow cloud metadata CIDRs. Hidden, dangerous, debug only.
    #[arg(long, global = true, hide = true)]
    pub allow_metadata: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Create config, cache dir, and an MCP bearer token.
    Init {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        cloud: bool,
    },
    /// Compile an OpenAPI document and register it.
    AddSpec {
        #[arg(long)]
        name: String,
        #[arg(long, conflicts_with = "file")]
        url: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        /// Absolute upstream origin. Required when the spec is a file and `servers` is relative.
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        insecure_http: bool,
        #[arg(long)]
        force: bool,
    },
    /// List registered specs.
    List,
    /// Show spec, tool, or client paste snippets.
    Inspect {
        name: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long, value_enum)]
        client: Option<ClientKind>,
    },
    /// Manage upstream credential references.
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Serve a spec over Streamable HTTP or stdio.
    Serve {
        name: String,
        #[arg(long)]
        stdio: bool,
        /// Public anonymous URL at https://<slug>.mcp.fetchhive.com/mcp.
        /// Uses the HTTP transport; do not combine with --stdio.
        #[arg(long, conflicts_with = "stdio")]
        tunnel: bool,
        /// Persistent name. Not available yet.
        #[arg(long = "name", value_name = "SLUG", hide = true, requires = "tunnel")]
        tunnel_name: Option<String>,
        /// How remote MCP clients authenticate. `public` needs --allow-anonymous.
        #[arg(long, value_enum, default_value_t = TunnelAuth::Token, requires = "tunnel")]
        tunnel_auth: TunnelAuth,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long, default_value = "/mcp")]
        path: String,
        #[arg(long)]
        expose: bool,
        #[arg(long)]
        allow_anonymous: bool,
        #[arg(long)]
        token_file: Option<PathBuf>,
        #[arg(long)]
        allow_insecure_http: bool,
        /// Absolute upstream origin. Overrides OpenAPI `servers` for this process.
        #[arg(long)]
        base_url: Option<String>,
        /// HTTPS OpenAPI document URL. Used when NAME is not in config (PaaS bootstrap).
        /// Overrides $MCP_GATEWAY_SPEC_URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Expose any Streamable HTTP or stdio MCP server through an anonymous tunnel.
    Tunnel {
        /// Upstream Streamable HTTP URL, for example http://127.0.0.1:8000/mcp.
        #[arg(
            value_name = "URL",
            required_unless_present = "stdio",
            conflicts_with = "stdio"
        )]
        url: Option<String>,
        /// Run a stdio MCP server. The command and its arguments follow `--`.
        #[arg(long, conflicts_with = "url")]
        stdio: bool,
        /// Command and arguments. Only valid after `--` and only with `--stdio`.
        #[arg(
            last = true,
            required = false,
            num_args = 1..,
            allow_hyphen_values = true,
            value_name = "CMD",
            requires = "stdio"
        )]
        command: Vec<String>,
        /// How remote clients authenticate to this proxy.
        #[arg(long, value_enum, default_value_t = ProxyAuth::Token)]
        tunnel_auth: ProxyAuth,
        /// Bearer token for token mode, or the upstream token sent on the passthrough probe.
        #[arg(long, env = "MCP_GATEWAY_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Local listen address for --stdio. Default 127.0.0.1:8787.
        #[arg(long, value_name = "ADDR", conflicts_with = "url")]
        bind: Option<String>,
        /// Persistent name. Not available yet.
        #[arg(long = "name", value_name = "SLUG", hide = true)]
        name: Option<String>,
        /// Skip the initialize and tools/list probe.
        #[arg(long)]
        no_probe: bool,
        /// Allow an upstream that resolves to a public address.
        #[arg(long)]
        allow_remote_upstream: bool,
    },
    /// Run local health checks.
    Doctor {
        name: Option<String>,
        #[arg(long)]
        offline: bool,
    },
    /// Call one tool through the same proxy path as serve.
    Test {
        name: String,
        tool: String,
        #[arg(long, default_value = "{}")]
        args: String,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        /// Absolute upstream origin. Overrides OpenAPI `servers` for this call.
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Read the local JSON log file.
    Logs {
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        tool: Option<String>,
    },
    /// Print build metadata.
    Version,
    /// Replace this binary from a GitHub Release.
    Upgrade {
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Compile an OpenAPI document to IR.
    #[command(hide = true)]
    Compile {
        spec: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// List tools in a compiled IR document.
    #[command(hide = true, name = "list-tools")]
    ListTools {
        ir: PathBuf,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Call one tool from a compiled IR document.
    #[command(hide = true)]
    Call {
        ir: PathBuf,
        tool_name: String,
        #[arg(long)]
        args: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, default_value = "MCP_GATEWAY_BEARER")]
        bearer_env: String,
        #[arg(long)]
        allow_disabled: bool,
    },
    /// Run the compile corpus.
    #[command(hide = true)]
    Corpus {
        #[arg(long)]
        only: Option<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthCmd {
    Add {
        name: String,
        #[arg(long, value_enum)]
        r#type: AuthType,
        #[arg(long)]
        header: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        from_env: Option<String>,
        #[arg(long)]
        from_file: Option<PathBuf>,
        #[arg(long = "from-env-header", value_name = "HEADER=VAR")]
        from_env_headers: Vec<String>,
    },
    List {
        name: Option<String>,
    },
    Remove {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AuthType {
    None,
    Bearer,
    Basic,
    #[clap(name = "api_key_header")]
    ApiKeyHeader,
    #[clap(name = "api_key_query")]
    ApiKeyQuery,
    #[clap(name = "custom_headers")]
    CustomHeaders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TunnelAuth {
    /// Remote clients must send the local MCP bearer token.
    Token,
    /// Remote clients may omit Authorization. Requires --allow-anonymous.
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProxyAuth {
    /// Require Authorization: Bearer and strip it before the upstream sees it.
    Token,
    /// Do not check Authorization. Forward the header upstream.
    Passthrough,
    /// Do not check Authorization. Strip it. The URL is public.
    Public,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ClientKind {
    Cursor,
    /// Claude Code (HTTP).
    #[clap(name = "claude-code")]
    ClaudeCode,
    /// Claude Desktop (stdio).
    Claude,
    Codex,
    Vscode,
    Chatgpt,
}

pub fn print_help_all() {
    println!(
        "mcp-gateway operator CLI plus hidden aliases.\n\n\
Visible commands:\n  init, add-spec, list, inspect, auth, serve, tunnel, doctor, test, logs, version, upgrade\n\n\
Hidden aliases (--help-all):\n  compile <SPEC> [--out ir.json] [--report report.json]\n  list-tools <ir.json> [--tag TAG]\n  call <ir.json> <tool_name> --args '<json>' [--base-url URL] [--bearer-env VAR] [--allow-disabled]\n  corpus [--only ID]\n"
    );
}
