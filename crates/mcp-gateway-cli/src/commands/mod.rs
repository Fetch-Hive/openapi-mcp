mod auth;
mod doctor;
mod hidden;
mod init;
mod logs;
mod serve;
mod spec;
mod test_cmd;
mod upgrade;
mod version;

use crate::cli::{Cli, Commands};
use crate::config::GatewayConfig;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;

pub async fn dispatch(cli: Cli, out: &Output) -> Result<ExitCode, CliError> {
    let paths = PlatformPaths::resolve(cli.globals.config.as_deref());
    match cli.command {
        Commands::Init { force, bind, cloud } => init::run(
            &paths,
            out,
            force,
            bind,
            cloud,
            cli.globals.allow_private_networks,
        ),
        Commands::AddSpec {
            name,
            url,
            file,
            insecure_http,
            force,
        } => {
            spec::add(
                &paths,
                &cli.globals,
                out,
                name,
                url,
                file,
                insecure_http,
                force,
            )
            .await
        }
        Commands::List => spec::list(&paths, out),
        Commands::Inspect { name, tool, client } => {
            spec::inspect(&paths, &cli.globals, out, name, tool, client)
        }
        Commands::Auth(cmd) => auth::run(&paths, out, cmd),
        Commands::Serve {
            name,
            stdio,
            bind,
            path,
            expose,
            allow_anonymous,
            token_file,
            allow_insecure_http,
        } => {
            serve::run(
                &paths,
                &cli.globals,
                out,
                name,
                stdio,
                bind,
                path,
                expose,
                allow_anonymous,
                token_file,
                allow_insecure_http,
            )
            .await
        }
        Commands::Doctor { name, offline } => {
            doctor::run(&paths, &cli.globals, out, name, offline).await
        }
        Commands::Test {
            name,
            tool,
            args,
            timeout,
        } => test_cmd::run(&paths, &cli.globals, out, name, tool, args, timeout).await,
        Commands::Logs {
            follow,
            since,
            tool,
        } => logs::run(&paths, out, follow, since, tool),
        Commands::Version => version::run(out),
        Commands::Upgrade { version, dry_run } => upgrade::run(out, version, dry_run).await,
        Commands::Compile {
            spec,
            out: ir,
            report,
        } => hidden::compile(out, spec, ir, report),
        Commands::ListTools { ir, tag } => hidden::list_tools(out, ir, tag),
        Commands::Call {
            ir,
            tool_name,
            args,
            base_url,
            bearer_env,
            allow_disabled,
        } => {
            hidden::call(
                &cli.globals,
                out,
                ir,
                tool_name,
                args,
                base_url,
                bearer_env,
                allow_disabled,
            )
            .await
        }
        Commands::Corpus { only } => hidden::corpus(out, only),
    }
}

pub fn load_cfg(paths: &PlatformPaths) -> Result<GatewayConfig, CliError> {
    if !paths.config_file.exists() {
        return Err(CliError::usage(format!(
            "no config at {}; run mcp-gateway init",
            paths.config_file.display()
        )));
    }
    GatewayConfig::load(&paths.config_file)
}
