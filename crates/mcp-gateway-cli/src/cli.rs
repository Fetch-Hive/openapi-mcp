use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "mcp-gateway",
    version,
    about = "Turn an OpenAPI document into a local MCP server.",
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
    /// Skip RFC1918/ULA denials and use the system resolver. Loud opt-in.
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
    /// Phase 1: compile an OpenAPI document to IR.
    #[command(hide = true)]
    Compile {
        spec: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Phase 1: list tools in a compiled IR document.
    #[command(hide = true, name = "list-tools")]
    ListTools {
        ir: PathBuf,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Phase 1: call one tool from a compiled IR document.
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
    /// Phase 1: run the compile corpus.
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ClientKind {
    Cursor,
    Claude,
    Vscode,
    Chatgpt,
}

pub fn print_help_all() {
    println!(
        "mcp-gateway operator CLI plus hidden Phase 1 aliases.\n\n\
Visible commands:\n  init, add-spec, list, inspect, auth, serve, doctor, test, logs, version, upgrade\n\n\
Hidden aliases (--help-all):\n  compile <SPEC> [--out ir.json] [--report report.json]\n  list-tools <ir.json> [--tag TAG]\n  call <ir.json> <tool_name> --args '<json>' [--base-url URL] [--bearer-env VAR] [--allow-disabled]\n  corpus [--only ID]\n"
    );
}
