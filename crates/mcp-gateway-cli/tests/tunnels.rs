use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const TOKEN: &str = "fh_cli_TUNNELFIXTURE";

fn bin() -> Command {
    let mut cmd = Command::cargo_bin("mcp-gateway").unwrap();
    cmd.env_remove("MCP_GATEWAY_CLI_TOKEN")
        .env_remove("MCP_GATEWAY_API_URL")
        .env_remove("MCP_GATEWAY_CONFIG")
        .env_remove("MCP_GATEWAY_TOKEN");
    cmd
}

fn write_credentials(dir: &std::path::Path) {
    let mut file = fs::File::create(dir.join("credentials.toml")).unwrap();
    writeln!(
        file,
        r#"[fetchhive]
api_url = "https://api.fetchhive.com"
token = "{TOKEN}"
account_id = "acct-file"
user_email = "file@example.com"
plan_type = "developer"
logged_in_at = "2026-09-26T00:00:00Z"
"#
    )
    .unwrap();
}

struct Seen {
    method: String,
    path: String,
    auth: String,
    body: String,
}

struct Hive {
    url: String,
    routes: Mutex<Vec<(String, String, u16, String)>>,
    seen: Mutex<Vec<Seen>>,
}

fn start(routes: Vec<(&str, &str, u16, &str)>) -> Arc<Hive> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hive = Arc::new(Hive {
        url: format!("http://127.0.0.1:{port}"),
        routes: Mutex::new(
            routes
                .into_iter()
                .map(|(method, path, status, body)| {
                    (method.to_owned(), path.to_owned(), status, body.to_owned())
                })
                .collect(),
        ),
        seen: Mutex::new(Vec::new()),
    });
    let shared = Arc::clone(&hive);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let shared = Arc::clone(&shared);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            if let Some(request) = read_http(&mut stream) {
                let (status, body) = shared.answer(&request);
                write_http(&mut stream, status, &body);
            }
        }
    });
    hive
}

impl Hive {
    fn answer(&self, request: &Parsed) -> (u16, String) {
        self.seen.lock().unwrap().push(Seen {
            method: request.method.clone(),
            path: request.path.clone(),
            auth: request.auth.clone(),
            body: request.body.clone(),
        });
        let mut routes = self.routes.lock().unwrap();
        if let Some(index) = routes
            .iter()
            .position(|(method, path, _, _)| method == &request.method && path == &request.path)
        {
            let (_, _, status, body) = routes.remove(index);
            return (status, body);
        }
        (404, r#"{"error":"not found"}"#.into())
    }
}

struct Parsed {
    method: String,
    path: String,
    auth: String,
    body: String,
}

fn read_http(stream: &mut std::net::TcpStream) -> Option<Parsed> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request: Vec<&str> = lines.next()?.split_whitespace().collect();
    let mut length = 0usize;
    let mut auth = String::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(0);
        }
        if name.eq_ignore_ascii_case("authorization") {
            auth = value.trim().to_string();
        }
    }
    while buf.len() < header_end + 4 + length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&buf[header_end + 4..]).to_string();
    Some(Parsed {
        method: request[0].to_string(),
        path: request[1].to_string(),
        auth,
        body,
    })
}

fn write_http(stream: &mut std::net::TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        _ => "Error",
    };
    let msg = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(msg.as_bytes());
}

fn me(used: u64, limit: u64) -> String {
    serde_json::json!({
        "account": {"id": "acct-1", "name": null, "plan_type": "developer"},
        "user": {"email": "ada@example.com"},
        "token": {"name": "laptop", "last_four": "TURE", "created_at": "2026-09-26T00:00:00Z"},
        "limits": {"tunnel_endpoints": {"used": used, "limit": limit}}
    })
    .to_string()
}

fn endpoint(slug: &str, online: bool) -> String {
    serde_json::json!({
        "slug": slug,
        "public_url": format!("https://{slug}.mcp.fetchhive.com/mcp"),
        "online": online,
        "last_connected_at": null,
        "created_at": "2026-09-26T00:00:00Z"
    })
    .to_string()
}

fn config_dir() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n").unwrap();
    write_credentials(dir.path());
    (dir, cfg.to_str().unwrap().to_owned())
}

#[test]
fn tunnels_need_login() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n").unwrap();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "tunnels", "list"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "`--name` needs a Fetch Hive login",
        ));
}

#[test]
fn list_prints_rows_and_quota_footer() {
    let row = endpoint("stripe-dev", false);
    let hive = start(vec![
        (
            "GET",
            "/v1/cli/tunnel_endpoints",
            200,
            &format!(r#"{{"mcp_tunnel_endpoints":[{row}]}}"#),
        ),
        ("GET", "/v1/cli/me", 200, &me(1, 1)),
    ]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stripe-dev"))
        .stdout(predicate::str::contains("offline"))
        .stdout(predicate::str::contains(
            "1/1 endpoints used · delete one or upgrade",
        ))
        .stdout(predicate::str::contains(TOKEN).not());
    let seen = hive.seen.lock().unwrap();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/v1/cli/tunnel_endpoints");
    assert_eq!(seen[0].auth, format!("Bearer {TOKEN}"));
}

#[test]
fn list_json_is_the_array() {
    let row = endpoint("stripe-dev", true);
    let hive = start(vec![(
        "GET",
        "/v1/cli/tunnel_endpoints",
        200,
        &format!(r#"{{"mcp_tunnel_endpoints":[{row}]}}"#),
    )]);
    let (_dir, cfg) = config_dir();
    let output = bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--json", "--config", &cfg, "tunnels", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(parsed.as_array().is_some(), "{parsed}");
    assert_eq!(parsed[0]["slug"], "stripe-dev");
    assert_eq!(parsed[0]["online"], true);
}

#[test]
fn empty_list_names_the_create_command() {
    let hive = start(vec![
        (
            "GET",
            "/v1/cli/tunnel_endpoints",
            200,
            r#"{"mcp_tunnel_endpoints":[]}"#,
        ),
        ("GET", "/v1/cli/me", 200, &me(0, 1)),
    ]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "No persistent tunnel names. Reserve one with `mcp-gateway tunnels create NAME`.",
        ))
        .stdout(predicate::str::contains("0/1 endpoints used"));
}

#[test]
fn create_posts_slug_and_prints_the_url() {
    let hive = start(vec![
        (
            "POST",
            "/v1/cli/tunnel_endpoints",
            201,
            &format!(
                r#"{{"mcp_tunnel_endpoint":{}}}"#,
                endpoint("stripe-dev", false)
            ),
        ),
        ("GET", "/v1/cli/me", 200, &me(1, 1)),
    ]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "create", "stripe-dev"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "✓ reserved stripe-dev for account Developer (1/1 endpoints used)",
        ))
        .stdout(predicate::str::contains(
            "https://stripe-dev.mcp.fetchhive.com/mcp",
        ))
        .stdout(predicate::str::contains(TOKEN).not());
    let seen = hive.seen.lock().unwrap();
    assert!(seen[0].body.contains("\"slug\":\"stripe-dev\""));
}

#[test]
fn delete_without_yes_is_refused_off_a_tty() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n").unwrap();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "tunnels",
            "delete",
            "stripe-dev",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "pass --yes to release a tunnel name without a prompt",
        ));
}

#[test]
fn delete_yes_releases_the_name() {
    let hive = start(vec![(
        "DELETE",
        "/v1/cli/tunnel_endpoints/stripe-dev",
        204,
        "",
    )]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "delete", "stripe-dev", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "released stripe-dev; a running tunnel using it will disconnect.",
        ));
}

#[test]
fn plan_limit_exits_2_with_the_server_upgrade_url() {
    let hive = start(vec![(
        "POST",
        "/v1/cli/tunnel_endpoints",
        402,
        r#"{"error_code":"mcp_tunnel_endpoint_limit","limit":1,"current":1,"upgrade_url":"https://app.fetchhive.com/checkout"}"#,
    )]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "create", "other-dev"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "this plan allows 1 persistent endpoints (1 in use). upgrade: https://app.fetchhive.com/checkout",
        ));
}

#[test]
fn revoked_login_leaves_the_credentials_file() {
    let hive = start(vec![(
        "GET",
        "/v1/cli/tunnel_endpoints",
        401,
        r#"{"error_code":"cli_token_invalid"}"#,
    )]);
    let (dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "list"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "login expired or revoked; run `mcp-gateway login`",
        ));
    assert!(dir.path().join("credentials.toml").exists());
}

#[test]
fn error_body_with_a_token_is_replaced() {
    let hive = start(vec![(
        "GET",
        "/v1/cli/tunnel_endpoints",
        500,
        r#"{"error":"fh_cli_LEAKED"}"#,
    )]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Fetch Hive returned a token on an error response",
        ))
        .stderr(predicate::str::contains("fh_cli_LEAKED").not());
}

#[test]
fn delete_missing_name_is_a_usage_error() {
    let hive = start(vec![(
        "DELETE",
        "/v1/cli/tunnel_endpoints/stripe-dev",
        404,
        r#"{"error":"not found"}"#,
    )]);
    let (_dir, cfg) = config_dir();
    bin()
        .env("MCP_GATEWAY_API_URL", &hive.url)
        .args(["--config", &cfg, "tunnels", "delete", "stripe-dev", "--yes"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("no tunnel named stripe-dev"));
}

#[test]
fn local_slug_rules_run_before_http() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n").unwrap();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "tunnels",
            "create",
            "admin",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("admin is reserved"));
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "tunnels",
            "create",
            "abcdefgh",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "abcdefgh looks like an anonymous tunnel id",
        ));
}
