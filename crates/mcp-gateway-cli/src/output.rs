use crate::cli::{ColorMode, Globals};
use serde::Serialize;

pub struct Output {
    pub json: bool,
    pub quiet: bool,
    pub verbose: u8,
}

impl Output {
    pub fn new(globals: &Globals) -> Self {
        match globals.color {
            ColorMode::Always => std::env::set_var("CLICOLOR_FORCE", "1"),
            ColorMode::Never => std::env::set_var("NO_COLOR", "1"),
            ColorMode::Auto => {}
        }
        Self {
            json: globals.json,
            quiet: globals.quiet,
            verbose: globals.verbose,
        }
    }

    pub fn line(&self, msg: &str) {
        if !self.quiet && !self.json {
            println!("{msg}");
        }
    }

    pub fn err_line(&self, msg: &str) {
        if !self.quiet {
            eprintln!("{msg}");
        }
    }

    pub fn error(&self, msg: &str) {
        if self.json {
            let v = serde_json::json!({ "error": msg });
            println!("{v}");
        } else {
            eprintln!("error: {msg}");
        }
    }

    pub fn json_value(&self, value: &impl Serialize) {
        let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
        println!("{rendered}");
    }

    pub fn verbose(&self, msg: &str) {
        if self.verbose > 0 && !self.quiet {
            eprintln!("{msg}");
        }
    }
}
