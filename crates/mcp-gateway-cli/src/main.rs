mod cli;
mod clients;
mod commands;
mod config;
mod exit;
mod ir_cache;
mod output;
mod paths;
mod runtime;
mod secrets;

use clap::Parser;
use cli::Cli;
use exit::ExitCode;
use output::Output;

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    if raw.iter().any(|a| a == "--help-all") {
        cli::print_help_all();
        std::process::exit(0);
    }

    let cli = Cli::parse();
    let out = Output::new(&cli.globals);
    let code = match run(cli, &out) {
        Ok(code) => code,
        Err(err) => {
            out.error(&err.to_string());
            err.exit_code()
        }
    };
    std::process::exit(code as i32);
}

fn run(cli: Cli, out: &Output) -> Result<ExitCode, CliError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::io(e.to_string()))?;
    rt.block_on(commands::dispatch(cli, out))
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Policy(String),
    #[error("{0}")]
    SupplyChain(String),
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    Io(String),
}

impl CliError {
    pub fn usage(msg: impl Into<String>) -> Self {
        Self::Usage(msg.into())
    }
    pub fn policy(msg: impl Into<String>) -> Self {
        Self::Policy(msg.into())
    }
    pub fn supply(msg: impl Into<String>) -> Self {
        Self::SupplyChain(msg.into())
    }
    pub fn upstream(msg: impl Into<String>) -> Self {
        Self::Upstream(msg.into())
    }
    pub fn io(msg: impl Into<String>) -> Self {
        Self::Io(msg.into())
    }
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Usage(_) | Self::Io(_) => ExitCode::Usage,
            Self::Policy(_) => ExitCode::Policy,
            Self::SupplyChain(_) => ExitCode::SupplyChain,
            Self::Upstream(_) => ExitCode::Upstream,
        }
    }
}
