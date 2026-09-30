use crate::cli::{ColorMode, Globals};
use serde::Serialize;
use std::io::IsTerminal;

pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const BOLD: &str = "\x1b[1m";
pub(crate) const GREEN: &str = "\x1b[32m";
pub(crate) const YELLOW: &str = "\x1b[33m";
pub(crate) const RED: &str = "\x1b[31m";
pub(crate) const DIM: &str = "\x1b[2m";
pub(crate) const CYAN: &str = "\x1b[36m";
pub(crate) const BLUE: &str = "\x1b[34m";
pub(crate) const MAGENTA: &str = "\x1b[35m";
pub(crate) const CYAN_UL: &str = "\x1b[36;4m";

pub(crate) fn paint_code(styled: bool, code: &str, text: &str) -> String {
    if styled && !code.is_empty() && !text.is_empty() {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

pub struct Output {
    pub json: bool,
    pub quiet: bool,
    pub verbose: u8,
    styled: bool,
}

impl Output {
    pub fn new(globals: &Globals) -> Self {
        match globals.color {
            ColorMode::Always => std::env::set_var("CLICOLOR_FORCE", "1"),
            ColorMode::Never => std::env::set_var("NO_COLOR", "1"),
            ColorMode::Auto => {}
        }
        let styled = match globals.color {
            ColorMode::Always => !globals.json,
            ColorMode::Never => false,
            ColorMode::Auto => {
                !globals.json
                    && std::io::stdout().is_terminal()
                    && std::env::var_os("NO_COLOR").is_none()
            }
        };
        Self {
            json: globals.json,
            quiet: globals.quiet,
            verbose: globals.verbose,
            styled,
        }
    }

    pub(crate) fn styled(&self) -> bool {
        self.styled
    }

    fn paint(&self, code: &str, text: &str) -> String {
        self.paint_with(code, text)
    }

    pub(crate) fn paint_with(&self, code: &str, text: &str) -> String {
        paint_code(self.styled, code, text)
    }

    pub fn bold(&self, text: &str) -> String {
        self.paint(BOLD, text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.paint(DIM, text)
    }

    pub fn heading(&self, text: &str) {
        self.line(&self.bold(text));
    }

    pub fn status_tag(&self, status: &str) -> String {
        let padded = format!("{status:<4}");
        let tag = format!("[{padded}]");
        match status {
            "ok" => self.paint(GREEN, &tag),
            "warn" => self.paint(YELLOW, &tag),
            "fail" => self.paint(RED, &tag),
            _ => tag,
        }
    }

    pub fn ok(&self, msg: &str) {
        let prefix = self.paint(GREEN, "✓");
        self.line(&format!("{prefix} {msg}"));
    }

    pub fn warn(&self, msg: &str) {
        let prefix = self.paint(YELLOW, "!");
        self.line(&format!("{prefix} {msg}"));
    }

    pub fn fail(&self, msg: &str) {
        let prefix = self.paint(RED, "✗");
        self.line(&format!("{prefix} {msg}"));
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
