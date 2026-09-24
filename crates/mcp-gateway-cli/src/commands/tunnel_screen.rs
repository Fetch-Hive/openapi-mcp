//! Human status screen for `serve --tunnel`.
//!
//! A terminal at least 11 rows tall, and wide enough for the auth line, pins
//! eight header rows with DECSTBM and scrolls request lines under them.
//! Any other stdout gets the same lines as normal text. `--json` and `--quiet`
//! draw nothing here.

use mcp_gateway_tunnel::{FinishedRequest, Stats, TunnelState};
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::output::Output;

pub const HEADER_ROWS: u16 = 8;
const LABEL_WIDTH: usize = 16;
const LEASE: &str = "anonymous, released 30 minutes after disconnect";

pub struct HeaderView {
    pub status: String,
    pub version: String,
    pub local_url: String,
    pub remote_url: String,
    pub auth: String,
    pub inflight: u32,
    pub total: u64,
    pub reconnects: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Plain,
    Pinned { rows: u16 },
}

pub struct TunnelScreen {
    mode: Mode,
    version: String,
    local_url: String,
    remote_url: String,
    public_auth: bool,
    status: String,
}

impl TunnelScreen {
    pub fn open(
        out: &Output,
        version: &str,
        local_url: &str,
        public_auth: bool,
        stats: &Stats,
    ) -> Self {
        let mut screen = Self {
            mode: Mode::Off,
            version: version.to_owned(),
            local_url: local_url.to_owned(),
            remote_url: "waiting".to_owned(),
            public_auth,
            status: status_text(&TunnelState::Connecting),
        };
        if out.json || out.quiet {
            return screen;
        }
        screen.mode = if io::stdout().is_terminal() {
            match terminal_size::terminal_size() {
                Some((terminal_size::Width(cols), terminal_size::Height(rows)))
                    if rows >= HEADER_ROWS + 3
                        && cols as usize > widest_header_line(public_auth) =>
                {
                    Mode::Pinned { rows }
                }
                _ => Mode::Plain,
            }
        } else {
            Mode::Plain
        };
        screen.paint_header(stats, true);
        screen
    }

    pub fn apply_state(&mut self, state: &TunnelState, stats: &Stats) {
        if let TunnelState::Connected { url, .. } = state {
            self.remote_url = url.clone();
        }
        self.status = status_text(state);
        if self.mode == Mode::Off {
            return;
        }
        self.paint_header(stats, false);
    }

    pub fn apply_request(&mut self, request: &FinishedRequest, stats: &Stats) {
        if self.mode == Mode::Off {
            return;
        }
        let line = request_line(&request.method, request.status, request.duration);
        match self.mode {
            Mode::Pinned { .. } => {
                let mut buf = line;
                buf.push('\n');
                write_stdout(&buf);
                self.redraw_rows(stats, &[HEADER_ROWS]);
            }
            Mode::Plain => {
                println!("{line}");
                println!("{}", self.lines(stats)[(HEADER_ROWS as usize) - 1]);
            }
            Mode::Off => {}
        }
    }

    pub fn finish(&mut self) {
        if let Mode::Pinned { rows } = self.mode {
            write_stdout(&format!("\x1b[r\x1b[{rows};1H"));
        }
        self.mode = Mode::Off;
    }

    fn paint_header(&self, stats: &Stats, full: bool) {
        let lines = self.lines(stats);
        match self.mode {
            Mode::Off => {}
            Mode::Plain => {
                if full {
                    for line in &lines {
                        println!("{line}");
                    }
                } else {
                    println!("{}", lines[0]);
                    println!("{}", lines[3]);
                    println!("{}", lines[(HEADER_ROWS as usize) - 1]);
                }
            }
            Mode::Pinned { rows } => {
                if full {
                    let mut buf = String::from("\x1b[H\x1b[2J");
                    for line in &lines {
                        buf.push_str(line);
                        buf.push('\n');
                    }
                    let start = HEADER_ROWS + 1;
                    buf.push_str(&format!("\x1b[{start};{rows}r\x1b[{start};1H"));
                    write_stdout(&buf);
                } else {
                    self.redraw_rows(stats, &[1, 4, HEADER_ROWS]);
                }
            }
        }
    }

    fn redraw_rows(&self, stats: &Stats, rows: &[u16]) {
        let lines = self.lines(stats);
        let mut buf = String::new();
        for row in rows {
            let text = &lines[(*row as usize) - 1];
            buf.push_str(&format!("\x1b7\x1b[{row};1H\x1b[2K{text}\x1b8"));
        }
        write_stdout(&buf);
    }

    fn lines(&self, stats: &Stats) -> [String; HEADER_ROWS as usize] {
        header_lines(&HeaderView {
            status: self.status.clone(),
            version: self.version.clone(),
            local_url: self.local_url.clone(),
            remote_url: self.remote_url.clone(),
            auth: auth_value(self.public_auth).to_owned(),
            inflight: stats.inflight.load(Ordering::SeqCst),
            total: stats.total.load(Ordering::SeqCst),
            reconnects: stats.reconnects.load(Ordering::SeqCst),
        })
    }
}

pub fn header_lines(view: &HeaderView) -> [String; HEADER_ROWS as usize] {
    [
        row("Session status", &view.status),
        row("Version", &view.version),
        row("Local URL", &view.local_url),
        row("Remote URL", &view.remote_url),
        row("Auth", &view.auth),
        row("Lease", LEASE),
        String::new(),
        row(
            "Requests",
            &format!(
                "in-flight {}    total {}    reconnects {}",
                view.inflight, view.total, view.reconnects
            ),
        ),
    ]
}

pub fn auth_value(public_auth: bool) -> &'static str {
    if public_auth {
        "public, this URL is reachable by anyone on the internet"
    } else {
        "bearer required"
    }
}

pub fn status_text(state: &TunnelState) -> String {
    match state {
        TunnelState::Connecting => "connecting".to_owned(),
        TunnelState::Connected { .. } => "online".to_owned(),
        TunnelState::Reconnecting { attempt, delay } => format!(
            "reconnecting (attempt {attempt}, next dial in {})",
            format_delay(*delay)
        ),
        TunnelState::Rejected { code, .. } => format!("rejected ({code})"),
        TunnelState::Stopped => "stopped".to_owned(),
    }
}

pub fn request_line(method: &str, status: u16, duration: Duration) -> String {
    format!("{method:<7} {status:>3} {}", format_delay(duration))
}

pub fn format_delay(duration: Duration) -> String {
    let ms = duration_ms(duration);
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

pub fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub fn local_http_url(authority: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("http://{authority}{path}")
    } else {
        format!("http://{authority}/{path}")
    }
}

fn widest_header_line(public_auth: bool) -> usize {
    LABEL_WIDTH
        + auth_value(public_auth)
            .chars()
            .count()
            .max(LEASE.chars().count())
}

fn row(label: &str, value: &str) -> String {
    format!("{label:<LABEL_WIDTH$}{value}")
}

fn write_stdout(text: &str) {
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HeaderView {
        HeaderView {
            status: "online".to_owned(),
            version: "0.7.1".to_owned(),
            local_url: "http://127.0.0.1:8787/mcp".to_owned(),
            remote_url: "https://abcd2345.mcp.fetchhive.com/mcp".to_owned(),
            auth: auth_value(false).to_owned(),
            inflight: 1,
            total: 4,
            reconnects: 2,
        }
    }

    #[test]
    fn header_names_status_version_urls_auth_lease_and_stats() {
        let lines = header_lines(&sample());
        assert_eq!(&lines[0][LABEL_WIDTH..], "online");
        assert_eq!(&lines[1][LABEL_WIDTH..], "0.7.1");
        assert_eq!(&lines[2][LABEL_WIDTH..], "http://127.0.0.1:8787/mcp");
        assert_eq!(
            &lines[3][LABEL_WIDTH..],
            "https://abcd2345.mcp.fetchhive.com/mcp"
        );
        assert_eq!(&lines[4][LABEL_WIDTH..], "bearer required");
        assert_eq!(
            &lines[5][LABEL_WIDTH..],
            "anonymous, released 30 minutes after disconnect"
        );
        assert_eq!(lines[6], "");
        assert_eq!(
            &lines[7][LABEL_WIDTH..],
            "in-flight 1    total 4    reconnects 2"
        );
        assert!(lines[0].starts_with("Session status"));
        assert!(lines[7].starts_with("Requests"));
    }

    #[test]
    fn public_auth_uses_the_open_tunnel_warning() {
        assert_eq!(
            auth_value(true),
            "public, this URL is reachable by anyone on the internet"
        );
    }

    #[test]
    fn reconnecting_status_names_attempt_and_delay() {
        let text = status_text(&TunnelState::Reconnecting {
            attempt: 0,
            delay: Duration::from_millis(1200),
        });
        assert_eq!(text, "reconnecting (attempt 0, next dial in 1.2s)");
    }

    #[test]
    fn request_line_is_method_status_and_duration() {
        assert_eq!(
            request_line("POST", 200, Duration::from_millis(12)),
            "POST    200 12ms"
        );
        assert_eq!(
            request_line("GET", 0, Duration::from_millis(5)),
            "GET       0 5ms"
        );
    }

    #[test]
    fn local_url_joins_authority_and_path() {
        assert_eq!(
            local_http_url("127.0.0.1:8787", "/mcp"),
            "http://127.0.0.1:8787/mcp"
        );
    }
}
