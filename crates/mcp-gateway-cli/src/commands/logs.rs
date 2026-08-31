use crate::commands::load_cfg;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;
use std::fs::File;
use std::io::{BufRead, BufReader};

pub fn run(
    paths: &PlatformPaths,
    out: &Output,
    follow: bool,
    since: Option<String>,
    tool: Option<String>,
) -> Result<ExitCode, CliError> {
    let cfg = load_cfg(paths).ok();
    let log_path = cfg
        .as_ref()
        .and_then(|c| {
            if c.log.file.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(&c.log.file))
            }
        })
        .unwrap_or_else(|| paths.log_file.clone());
    if follow {
        return Err(CliError::usage(
            "--follow is not implemented in this build; rerun logs without --follow",
        ));
    }
    if !log_path.exists() {
        return Err(CliError::usage(format!(
            "no log file at {}; run mcp-gateway serve first",
            log_path.display()
        )));
    }
    let file = File::open(&log_path).map_err(|e| CliError::io(e.to_string()))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|e| CliError::io(e.to_string()))?;
        if let Some(tool_name) = &tool {
            if !line.contains(tool_name) {
                continue;
            }
        }
        if let Some(since) = &since {
            if line < *since {
                continue;
            }
        }
        out.line(&line);
    }
    Ok(ExitCode::Ok)
}
