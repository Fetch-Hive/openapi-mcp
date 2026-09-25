use assert_cmd::Command;
use predicates::prelude::*;
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ACCESS_TOKEN: &str = "fh_cli_SECRETTOKEN";
const DEVICE_CODE: &str = "devcode-SECRET-value";
const FILE_TOKEN: &str = "fh_cli_FILETOKEN";
const ENV_TOKEN: &str = "fh_cli_ENVTOKEN";

fn bin() -> Command {
    let mut cmd = Command::cargo_bin("mcp-gateway").unwrap();
    cmd.env_remove("MCP_GATEWAY_TEST_BASE_URL")
        .env_remove("MCP_GATEWAY_TEST_ALLOW_LOOPBACK")
        .env_remove("PORT")
        .env_remove("MCP_GATEWAY_SPEC_URL")
        .env_remove("MCP_GATEWAY_CLI_TOKEN")
        .env_remove("MCP_GATEWAY_API_URL")
        .env_remove("MCP_GATEWAY_CONFIG");
    cmd
}

fn config_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write_credentials(dir: &std::path::Path, token: &str) {
    let path = dir.join("credentials.toml");
    let mut file = fs::File::create(&path).unwrap();
    writeln!(
        file,
        r#"[fetchhive]
api_url = "https://api.fetchhive.com"
token = "{token}"
account_id = "acct-file"
user_email = "file@example.com"
plan_type = "developer"
logged_in_at = "2026-09-26T00:00:00Z"
"#
    )
    .unwrap();
}

struct Hive {
    url: String,
    polls: Mutex<VecDeque<(u16, String)>>,
    code_status: u16,
    code_body: String,
    me_status: u16,
    me_body: String,
    delete_status: u16,
    auths: Mutex<Vec<(String, String)>>,
    me_hits: AtomicUsize,
    delete_hits: AtomicUsize,
}

fn code_body(base: &str) -> String {
    serde_json::json!({
        "device_code": DEVICE_CODE,
        "user_code": "ABCD-EFGH",
        "verification_uri": format!("{base}/cli/authorize"),
        "verification_uri_complete": format!("{base}/cli/authorize?user_code=ABCD-EFGH"),
        "expires_in": 30,
        "interval": 0
    })
    .to_string()
}

fn grant_body() -> String {
    serde_json::json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "bearer",
        "account": {"id": "acct-1", "name": null, "plan_type": "developer"},
        "user": {"email": "ada@example.com"}
    })
    .to_string()
}

fn me_body(email: &str) -> String {
    serde_json::json!({
        "account": {"id": "acct-1", "name": null, "plan_type": "developer"},
        "user": {"email": email},
        "token": {
            "name": "laptop (darwin)",
            "last_four": "oken",
            "created_at": "2026-09-26T00:00:00Z"
        },
        "limits": {"tunnel_endpoints": {"used": 0, "limit": 1}}
    })
    .to_string()
}

fn start_hive(
    polls: Vec<(u16, String)>,
    code_status: u16,
    me_status: u16,
    me_email: &str,
) -> Arc<Hive> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    let hive = Arc::new(Hive {
        url: url.clone(),
        polls: Mutex::new(VecDeque::from(polls)),
        code_status,
        code_body: code_body(&url),
        me_status,
        me_body: me_body(me_email),
        delete_status: 204,
        auths: Mutex::new(Vec::new()),
        me_hits: AtomicUsize::new(0),
        delete_hits: AtomicUsize::new(0),
    });
    let shared = Arc::clone(&hive);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let shared = Arc::clone(&shared);
            thread::spawn(move || respond(stream, &shared));
        }
    });
    hive
}

fn respond(mut stream: TcpStream, hive: &Hive) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Some(request) = read_http(&mut stream) else {
        return;
    };
    let auth = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    hive.auths
        .lock()
        .unwrap()
        .push((request.method.clone(), auth));
    let (status, body) = match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/v1/anonymous/cli_device/code") => (hive.code_status, hive.code_body.clone()),
        ("POST", "/v1/anonymous/cli_device/token") => hive
            .polls
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((500, r#"{"error":"unexpected poll"}"#.into())),
        ("GET", "/v1/cli/me") => {
            hive.me_hits.fetch_add(1, Ordering::SeqCst);
            (hive.me_status, hive.me_body.clone())
        }
        ("DELETE", "/v1/cli/token") => {
            hive.delete_hits.fetch_add(1, Ordering::SeqCst);
            (hive.delete_status, String::new())
        }
        _ => (404, r#"{"error":"not found"}"#.into()),
    };
    write_http(&mut stream, status, &body);
}

struct ParsedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

fn read_http(stream: &mut TcpStream) -> Option<ParsedRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request: Vec<&str> = lines.next()?.split_whitespace().collect();
    if request.len() < 2 {
        return None;
    }
    let mut headers = Vec::new();
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(0);
        }
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    while buf.len() < header_end + 4 + length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Some(ParsedRequest {
        method: request[0].to_string(),
        path: request[1].to_string(),
        headers,
    })
}

fn write_http(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let msg = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(msg.as_bytes());
}

fn rfc(error: &str) -> (u16, String) {
    (400, format!(r#"{{"error":"{error}"}}"#))
}

#[test]
fn login_writes_private_credentials_and_whoami_reads_them() {
    let hive = start_hive(
        vec![
            rfc("authorization_pending"),
            rfc("authorization_pending"),
            (200, grant_body()),
        ],
        200,
        200,
        "ada@example.com",
    );
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n").unwrap();

    let output = bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "login",
            "--no-browser",
            "--api-url",
            &hive.url,
            "--force",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("ABCD-EFGH"), "{text}");
    assert!(text.contains("ada@example.com"), "{text}");
    assert!(
        text.contains("\"event\": \"logged_in\"") || text.contains("\"event\":\"logged_in\""),
        "{text}"
    );
    assert!(!text.contains(DEVICE_CODE), "{text}");
    assert!(!text.contains(ACCESS_TOKEN), "{text}");
    assert!(!text.contains("fh_cli_"), "{text}");

    let creds = dir.path().join("credentials.toml");
    let mode = fs::metadata(&creds).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let raw = fs::read_to_string(&creds).unwrap();
    assert!(raw.contains(ACCESS_TOKEN));
    assert!(raw.contains("ada@example.com"));

    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "whoami",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ada@example.com"))
        .stdout(predicate::str::contains("0/1"))
        .stdout(predicate::str::contains(ACCESS_TOKEN).not());
    assert_eq!(hive.me_hits.load(Ordering::SeqCst), 2);
    let auths = hive.auths.lock().unwrap().clone();
    assert!(auths
        .iter()
        .any(|(method, auth)| { method == "GET" && auth == &format!("Bearer {ACCESS_TOKEN}") }));

    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "logout",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Logged out."))
        .stdout(predicate::str::contains(ACCESS_TOKEN).not());
    assert_eq!(hive.delete_hits.load(Ordering::SeqCst), 1);
    assert!(!creds.exists());
}

#[test]
fn slow_down_waits_five_seconds_before_the_grant() {
    let hive = start_hive(
        vec![rfc("slow_down"), (200, grant_body())],
        200,
        200,
        "ada@example.com",
    );
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    let started = Instant::now();
    let output = bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "login",
            "--no-browser",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(started.elapsed() >= Duration::from_secs(5));
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("\"event\": \"device_code\"") || text.contains("\"event\":\"device_code\"")
    );
    assert!(text.contains("ABCD-EFGH"));
    assert!(!text.contains(DEVICE_CODE), "{text}");
    assert!(!text.contains(ACCESS_TOKEN), "{text}");
}

#[test]
fn login_denied_and_expired_do_not_write_a_file() {
    let hive = start_hive(vec![rfc("access_denied")], 200, 200, "ada@example.com");
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "login",
            "--no-browser",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("login denied in browser"))
        .stdout(predicate::str::contains(DEVICE_CODE).not())
        .stderr(predicate::str::contains(DEVICE_CODE).not());
    assert!(!dir.path().join("credentials.toml").exists());

    let hive = start_hive(vec![rfc("expired_token")], 200, 200, "ada@example.com");
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "login",
            "--no-browser",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("code expired; run login again"));
    assert!(!dir.path().join("credentials.toml").exists());
}

#[test]
fn login_unavailable_mentions_anonymous_tunnels() {
    let hive = start_hive(vec![], 503, 200, "ada@example.com");
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "login",
            "--no-browser",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .failure()
        .code(4)
        .stderr(predicate::str::contains(
            "local commands and anonymous tunnels still work",
        ));
}

#[test]
fn rate_limit_includes_retry_after() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    thread::spawn(move || {
        if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
            let _ = read_http(&mut stream);
            let body = r#"{"error_code":"cli_device_rate_limited","error":"slow"}"#;
            let msg = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(msg.as_bytes());
        }
    });
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "login",
            "--no-browser",
            "--api-url",
            &url,
        ])
        .assert()
        .failure()
        .code(4)
        .stderr(predicate::str::contains("Retry after 7 seconds"));
}

#[test]
fn whoami_uses_env_token_over_the_file() {
    let hive = start_hive(vec![], 200, 200, "env@example.com");
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "").unwrap();
    write_credentials(dir.path(), FILE_TOKEN);
    bin()
        .env("MCP_GATEWAY_CLI_TOKEN", ENV_TOKEN)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "whoami",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("env@example.com"))
        .stdout(predicate::str::contains(ENV_TOKEN).not())
        .stdout(predicate::str::contains(FILE_TOKEN).not());
    let auths = hive.auths.lock().unwrap().clone();
    assert!(auths
        .iter()
        .any(|(method, auth)| { method == "GET" && auth == &format!("Bearer {ENV_TOKEN}") }));
    assert!(dir.path().join("credentials.toml").exists());
}

#[test]
fn logout_keeps_the_file_when_the_token_comes_from_the_environment() {
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    write_credentials(dir.path(), FILE_TOKEN);
    bin()
        .env("MCP_GATEWAY_CLI_TOKEN", ENV_TOKEN)
        .args(["--config", cfg.to_str().unwrap(), "logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MCP_GATEWAY_CLI_TOKEN"))
        .stdout(predicate::str::contains(ENV_TOKEN).not());
    assert!(dir.path().join("credentials.toml").exists());
}

#[test]
fn logout_leaves_the_file_when_the_api_is_unreachable() {
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    write_credentials(dir.path(), FILE_TOKEN);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "logout",
            "--api-url",
            &format!("http://127.0.0.1:{port}"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("MCP_GATEWAY_API_URL"))
        .stderr(predicate::str::contains(FILE_TOKEN).not());
    assert!(dir.path().join("credentials.toml").exists());
}

#[test]
fn whoami_not_logged_in_exits_zero() {
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "--json", "whoami"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logged_in\": false"));
}

#[test]
fn whoami_clear_removes_a_revoked_file() {
    let hive = start_hive(vec![], 200, 401, "ada@example.com");
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    write_credentials(dir.path(), FILE_TOKEN);
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "whoami",
            "--clear",
            "--api-url",
            &hive.url,
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Token revoked or expired"))
        .stderr(predicate::str::contains(FILE_TOKEN).not());
    assert!(!dir.path().join("credentials.toml").exists());
}

#[test]
fn login_without_force_prints_the_stored_identity() {
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    write_credentials(dir.path(), FILE_TOKEN);
    bin()
        .args(["--config", cfg.to_str().unwrap(), "login"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Already logged in as file@example.com",
        ))
        .stdout(predicate::str::contains("2026-09-26T00:00:00Z"))
        .stdout(predicate::str::contains(FILE_TOKEN).not());
}

#[test]
fn doctor_warns_when_credentials_are_group_readable() {
    let dir = config_dir();
    let cfg = dir.path().join("config.toml");
    let init = bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&init);
    let bearer = stdout
        .lines()
        .find(|line| line.contains("export MCP_GATEWAY_TOKEN="))
        .and_then(|line| line.split('=').nth(1))
        .map(|value| value.split_whitespace().next().unwrap_or("").to_string())
        .unwrap_or_default();
    let creds = dir.path().join("credentials.toml");
    fs::write(
        &creds,
        r#"[fetchhive]
api_url = "https://api.fetchhive.com"
token = "fh_cli_FILETOKEN"
account_id = "acct-file"
user_email = "file@example.com"
plan_type = "developer"
logged_in_at = "2026-09-26T00:00:00Z"
"#,
    )
    .unwrap();
    fs::set_permissions(&creds, fs::Permissions::from_mode(0o644)).unwrap();
    bin()
        .env("MCP_GATEWAY_TOKEN", bearer)
        .args(["--config", cfg.to_str().unwrap(), "doctor", "--offline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("group or world readable"))
        .stdout(predicate::str::contains("file@example.com"))
        .stdout(predicate::str::contains(FILE_TOKEN).not());
}
