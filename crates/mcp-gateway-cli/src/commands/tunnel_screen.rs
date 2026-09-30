//! Human status screen for `serve --tunnel` and `tunnel`.
//!
//! A terminal at least three rows taller than the header, and wide enough for
//! the longest header line, pins the header with DECSTBM and scrolls request
//! lines under it. `serve --tunnel` has eight header rows. `tunnel` inserts
//! extra rows between Auth and Lease. Any other stdout gets the same lines as
//! normal text. `--json` and `--quiet` draw nothing here. A finished call
//! prints `METHOD  STATUS DURATION  PATH  RPC`.

use mcp_gateway_tunnel::{FinishedRequest, Stats, TunnelState};
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::output::{
    paint_code, Output, BLUE, BOLD, CYAN, CYAN_UL, DIM, GREEN, MAGENTA, RED, RESET, YELLOW,
};

pub const HEADER_ROWS: u16 = 8;
const LABEL_WIDTH: usize = 16;
const LEASE: &str = "anonymous, released 30 minutes after disconnect";
const PERSISTENT_LEASE: &str = "persistent, stays reserved while offline";

pub struct HeaderView {
    pub status: String,
    pub version: String,
    pub local_url: String,
    pub remote_url: String,
    pub auth: String,
    pub lease: String,
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
    remote_note: String,
    auth: String,
    lease: String,
    extras: Vec<(String, String)>,
    status: String,
    styled: bool,
}

impl TunnelScreen {
    pub fn open(
        out: &Output,
        version: &str,
        local_url: &str,
        public_auth: bool,
        auth_override: Option<&str>,
        extras: &[(&str, &str)],
        stats: &Stats,
    ) -> Self {
        let auth = auth_override
            .unwrap_or_else(|| auth_value(public_auth))
            .to_owned();
        let extras = extras
            .iter()
            .map(|(label, value)| ((*label).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        let mut screen = Self {
            mode: Mode::Off,
            version: version.to_owned(),
            local_url: local_url.to_owned(),
            remote_url: "waiting".to_owned(),
            remote_note: String::new(),
            auth,
            lease: LEASE.to_owned(),
            extras,
            status: status_text(&TunnelState::Connecting),
            styled: out.styled(),
        };
        if out.json || out.quiet {
            return screen;
        }
        let header_rows = screen.header_rows();
        screen.mode = if io::stdout().is_terminal() {
            match terminal_size::terminal_size() {
                Some((terminal_size::Width(cols), terminal_size::Height(rows)))
                    if rows >= header_rows + 3 && cols as usize > screen.widest() =>
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

    pub fn persistent(mut self, label: &str) -> Self {
        self.lease = PERSISTENT_LEASE.to_owned();
        self.remote_note = format!("  (persistent — {label})");
        self
    }

    pub fn apply_state(&mut self, state: &TunnelState, stats: &Stats) {
        if let TunnelState::Connected { url, .. } = state {
            self.remote_url = format!("{url}{}", self.remote_note);
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
        let line = if self.styled {
            paint_request_line(
                true,
                &request.method,
                request.status,
                request.duration,
                &request.path,
                request.rpc.as_deref(),
            )
        } else {
            request_line(
                &request.method,
                request.status,
                request.duration,
                &request.path,
                request.rpc.as_deref(),
            )
        };
        match self.mode {
            Mode::Pinned { .. } => {
                let mut buf = line;
                buf.push('\n');
                write_stdout(&buf);
                self.redraw_rows(stats, &[self.header_rows()]);
            }
            Mode::Plain => {
                println!("{line}");
                if let Some(last) = self.lines(stats).last() {
                    println!("{last}");
                }
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

    fn header_rows(&self) -> u16 {
        HEADER_ROWS + u16::try_from(self.extras.len()).unwrap_or(0)
    }

    fn widest(&self) -> usize {
        let mut width = LABEL_WIDTH + self.auth.chars().count().max(self.lease.chars().count());
        for (label, value) in &self.extras {
            width = width.max(row(label, value).chars().count());
        }
        width
    }

    fn paint_header(&self, stats: &Stats, full: bool) {
        let lines = self.lines(stats);
        let count = lines.len() as u16;
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
                    println!("{}", lines[(count as usize) - 1]);
                }
            }
            Mode::Pinned { rows } => {
                if full {
                    let mut buf = String::from("\x1b[H\x1b[2J");
                    for line in &lines {
                        buf.push_str(line);
                        buf.push('\n');
                    }
                    let start = count + 1;
                    buf.push_str(&format!("\x1b[{start};{rows}r\x1b[{start};1H"));
                    write_stdout(&buf);
                } else {
                    self.redraw_rows(stats, &[1, 4, count]);
                }
            }
        }
    }

    fn redraw_rows(&self, stats: &Stats, rows: &[u16]) {
        let lines = self.lines(stats);
        let mut buf = String::new();
        for row_number in rows {
            let text = &lines[(*row_number as usize) - 1];
            buf.push_str(&format!("\x1b7\x1b[{row_number};1H\x1b[2K{text}\x1b8"));
        }
        write_stdout(&buf);
    }

    fn lines(&self, stats: &Stats) -> Vec<String> {
        let extras = self
            .extras
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let lines = screen_lines(
            &HeaderView {
                status: self.status.clone(),
                version: self.version.clone(),
                local_url: self.local_url.clone(),
                remote_url: self.remote_url.clone(),
                auth: self.auth.clone(),
                lease: self.lease.clone(),
                inflight: stats.inflight.load(Ordering::SeqCst),
                total: stats.total.load(Ordering::SeqCst),
                reconnects: stats.reconnects.load(Ordering::SeqCst),
            },
            &extras,
        );
        if !self.styled {
            return lines;
        }
        lines
            .into_iter()
            .map(|line| color_header_line(&line))
            .collect()
    }
}

pub fn screen_lines(view: &HeaderView, extras: &[(&str, &str)]) -> Vec<String> {
    let base = header_lines(view);
    let mut lines = Vec::with_capacity(base.len() + extras.len());
    lines.extend(base[..5].iter().cloned());
    for (label, value) in extras {
        lines.push(row(label, value));
    }
    lines.extend(base[5..].iter().cloned());
    lines
}

pub fn header_lines(view: &HeaderView) -> [String; HEADER_ROWS as usize] {
    [
        row("Session status", &view.status),
        row("Version", &view.version),
        row("Local URL", &view.local_url),
        row("Remote URL", &view.remote_url),
        row("Auth", &view.auth),
        row("Lease", &view.lease),
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

pub fn request_line(
    method: &str,
    status: u16,
    duration: Duration,
    path: &str,
    rpc: Option<&str>,
) -> String {
    paint_request_line(false, method, status, duration, path, rpc)
}

pub fn paint_request_line(
    styled: bool,
    method: &str,
    status: u16,
    duration: Duration,
    path: &str,
    rpc: Option<&str>,
) -> String {
    let method_col = format!("{method:<4}");
    let status_col = format!("{status:>3}");
    let dur_col = format!("{:>5}", format_delay(duration));
    let method_col = paint_code(styled, BOLD, &method_col);
    let status_col = paint_code(styled, status_color(status), &status_col);
    let dur_code = if duration >= Duration::from_secs(1) {
        YELLOW
    } else {
        DIM
    };
    let dur_col = paint_code(styled, dur_code, &dur_col);
    let mut line = format!("{method_col}  {status_col} {dur_col}  {path}");
    if let Some(rpc) = rpc.filter(|label| !label.is_empty()) {
        line.push_str("  ");
        line.push_str(&paint_rpc(styled, rpc));
    }
    line
}

fn status_color(status: u16) -> &'static str {
    match status {
        0 => DIM,
        429 => RED,
        200..=299 => GREEN,
        300..=399 => CYAN,
        400..=499 => YELLOW,
        500..=599 => RED,
        _ => "",
    }
}

fn paint_rpc(styled: bool, rpc: &str) -> String {
    const PREFIX: &str = "tools/call ";
    let Some(rest) = rpc.strip_prefix(PREFIX) else {
        return paint_code(styled, CYAN, rpc);
    };
    let (name, extra) = batch_suffix(rest);
    if name.is_empty() {
        return paint_code(styled, CYAN, rpc);
    }
    if !styled {
        return rpc.to_owned();
    }
    // `\x1b[1m` sets bold and leaves the cyan from the prefix in place.
    let mut line = format!("{CYAN}{PREFIX}{BOLD}{name}{RESET}");
    if !extra.is_empty() {
        line.push_str(&paint_code(true, CYAN, extra));
    }
    line
}

/// Split `list_issues +2` into the tool name and the ` +N` batch suffix.
fn batch_suffix(rest: &str) -> (&str, &str) {
    let Some(idx) = rest.rfind(" +") else {
        return (rest, "");
    };
    let suffix = &rest[idx + 2..];
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return (rest, "");
    }
    (&rest[..idx], &rest[idx..])
}

fn color_header_line(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }
    let mut chars = line.chars();
    let label: String = chars.by_ref().take(LABEL_WIDTH).collect();
    let value: String = chars.collect();
    let painted = color_header_value(label.trim_end(), &value);
    format!("{}{painted}", paint_code(true, DIM, &label))
}

fn color_header_value(label: &str, value: &str) -> String {
    match label {
        "Session status" => {
            let code = if value == "online" {
                GREEN
            } else if value == "stopped" || value.starts_with("rejected") {
                RED
            } else if value == "connecting" || value.starts_with("reconnecting") {
                YELLOW
            } else {
                return value.to_owned();
            };
            paint_code(true, code, value)
        }
        "Version" => paint_code(true, MAGENTA, value),
        "Local URL" => paint_code(true, BLUE, value),
        "Remote URL" => paint_remote(value),
        "Auth" if value.starts_with("public") => paint_code(true, YELLOW, value),
        _ => value.to_owned(),
    }
}

fn paint_remote(value: &str) -> String {
    let url_end = value.find(char::is_whitespace).unwrap_or(value.len());
    let (url, rest) = value.split_at(url_end);
    if url.starts_with("http://") || url.starts_with("https://") {
        format!(
            "{}{}",
            paint_code(true, CYAN_UL, url),
            paint_code(true, DIM, rest)
        )
    } else {
        value.to_owned()
    }
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
            lease: LEASE.to_owned(),
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
    fn persistent_lease_stays_reserved_while_offline() {
        let mut view = sample();
        view.lease = PERSISTENT_LEASE.to_owned();
        let lines = header_lines(&view);
        assert_eq!(&lines[5][LABEL_WIDTH..], PERSISTENT_LEASE);
    }

    #[test]
    fn extra_rows_sit_between_auth_and_lease() {
        let lines = screen_lines(
            &sample(),
            &[
                ("Upstream", "http://127.0.0.1:8000/mcp"),
                ("Probe", "fixture 0, 1 tools"),
            ],
        );
        assert_eq!(lines.len(), 10);
        assert_eq!(&lines[4][LABEL_WIDTH..], "bearer required");
        assert_eq!(&lines[5][LABEL_WIDTH..], "http://127.0.0.1:8000/mcp");
        assert_eq!(&lines[6][LABEL_WIDTH..], "fixture 0, 1 tools");
        assert!(lines[7].starts_with("Lease"));
        assert_eq!(lines[8], "");
        assert!(lines[9].starts_with("Requests"));
        assert_eq!(
            screen_lines(&sample(), &[]),
            header_lines(&sample()).to_vec()
        );
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
    fn request_line_is_method_status_duration_path_and_rpc() {
        assert_eq!(
            request_line(
                "POST",
                200,
                Duration::from_millis(11),
                "/mcp",
                Some("tools/call list_issues")
            ),
            "POST  200  11ms  /mcp  tools/call list_issues"
        );
        assert_eq!(
            request_line(
                "POST",
                200,
                Duration::from_millis(4),
                "/mcp",
                Some("tools/list")
            ),
            "POST  200   4ms  /mcp  tools/list"
        );
        assert_eq!(
            request_line("GET", 0, Duration::from_millis(5), "/mcp", None),
            "GET     0   5ms  /mcp"
        );
    }

    #[test]
    fn request_line_colors_follow_status_and_leave_plain_text_when_unstyled() {
        let plain = request_line(
            "POST",
            200,
            Duration::from_millis(11),
            "/mcp",
            Some("tools/call list_issues"),
        );
        let colored = paint_request_line(
            true,
            "POST",
            200,
            Duration::from_millis(11),
            "/mcp",
            Some("tools/call list_issues"),
        );
        assert_eq!(visible(&colored), plain);
        assert!(colored.contains("\x1b[1mPOST\x1b[0m"));
        assert!(colored.contains("\x1b[32m200\x1b[0m"));
        assert!(colored.contains("\x1b[2m 11ms\x1b[0m"));
        assert!(colored.contains("\x1b[36mtools/call \x1b[1mlist_issues\x1b[0m"));
        let slow = paint_request_line(true, "POST", 429, Duration::from_millis(1200), "/mcp", None);
        assert!(slow.contains("\x1b[31m429\x1b[0m"));
        assert!(slow.contains("\x1b[33m 1.2s\x1b[0m"));
        let cancelled = paint_request_line(true, "GET", 0, Duration::from_millis(5), "/mcp", None);
        assert!(cancelled.contains("\x1b[2m  0\x1b[0m"));
    }

    #[test]
    fn header_colors_do_not_change_the_visible_row() {
        let plain = header_lines(&sample());
        let online = color_header_line(&plain[0]);
        assert_eq!(visible(&online), plain[0]);
        assert!(online.contains("\x1b[32monline\x1b[0m"));
        let remote = color_header_line(&plain[3]);
        assert_eq!(visible(&remote), plain[3]);
        assert!(remote.contains("\x1b[36;4mhttps://abcd2345.mcp.fetchhive.com/mcp\x1b[0m"));
        let version = color_header_line(&plain[1]);
        assert!(version.contains("\x1b[35m0.7.1\x1b[0m"));
        let local = color_header_line(&plain[2]);
        assert!(local.contains("\x1b[34mhttp://127.0.0.1:8787/mcp\x1b[0m"));
        let mut view = sample();
        view.status = "connecting".to_owned();
        view.auth = auth_value(true).to_owned();
        let lines = header_lines(&view);
        assert!(color_header_line(&lines[0]).contains("\x1b[33mconnecting\x1b[0m"));
        assert!(color_header_line(&lines[4]).contains("\x1b[33mpublic,"));
        view.status = "stopped".to_owned();
        assert!(color_header_line(&header_lines(&view)[0]).contains("\x1b[31mstopped\x1b[0m"));
    }

    fn visible(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn local_url_joins_authority_and_path() {
        assert_eq!(
            local_http_url("127.0.0.1:8787", "/mcp"),
            "http://127.0.0.1:8787/mcp"
        );
    }
}
