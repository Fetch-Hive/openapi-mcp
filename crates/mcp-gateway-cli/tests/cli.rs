use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use tempfile::TempDir;

fn bin() -> Command {
    let mut cmd = Command::cargo_bin("mcp-gateway").unwrap();
    // Debug-only hooks; must not leak from the operator's shell into CLI tests.
    cmd.env_remove("MCP_GATEWAY_TEST_BASE_URL")
        .env_remove("MCP_GATEWAY_TEST_ALLOW_LOOPBACK")
        .env_remove("PORT")
        .env_remove("MCP_GATEWAY_SPEC_URL");
    cmd
}

fn primed() -> (TempDir, std::path::PathBuf, String) {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    let init = bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&init);
    let token = stdout
        .lines()
        .find(|l| l.contains("export MCP_GATEWAY_TOKEN="))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.split_whitespace().next().unwrap().to_string())
        .unwrap_or_default();
    let spec = dir.path().join("tiny.yaml");
    fs::copy("tests/fixtures/tiny.yaml", &spec).unwrap();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "add-spec",
            "--name",
            "petstore",
            "--file",
            spec.to_str().unwrap(),
        ])
        .assert()
        .success();
    (dir, cfg, token)
}

#[test]
fn help_hides_compile() {
    bin()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("compile").not());
}

#[test]
fn help_all_lists_hidden() {
    bin()
        .arg("--help-all")
        .assert()
        .success()
        .stdout(predicate::str::contains("compile <SPEC>"))
        .stdout(predicate::str::contains("list-tools"));
}

#[test]
fn init_then_exists_fails() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MCP_GATEWAY_TOKEN"));
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("config already exists"));
}

#[test]
fn add_spec_file_and_list() {
    let (_dir, cfg, _) = primed();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("petstore"));
}

#[test]
fn add_spec_ssrf_metadata() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "add-spec",
            "--name",
            "internal",
            "--url",
            "https://metadata.google.internal/openapi.json",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("spec URL rejected"));
}

#[test]
fn auth_add_missing_env() {
    let (_dir, cfg, _) = primed();
    bin()
        .env_remove("PETSTORE_TOKEN")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "auth",
            "add",
            "petstore",
            "--type",
            "bearer",
            "--from-env",
            "PETSTORE_TOKEN_MISSING_XYZ",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("is not set"));
}

#[test]
fn auth_list_json_has_no_secret_literals() {
    let (_dir, cfg, _) = primed();
    bin()
        .env("PETSTORE_TOKEN", "fh_mcp_live_should_never_appear")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "auth",
            "add",
            "petstore",
            "--type",
            "bearer",
            "--from-env",
            "PETSTORE_TOKEN",
        ])
        .assert()
        .success();
    let output = bin()
        .args(["--config", cfg.to_str().unwrap(), "--json", "auth", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&output);
    assert!(
        !text.contains("fh_mcp_live_should_never_appear"),
        "secret leaked: {text}"
    );
    assert!(text.contains("env:PETSTORE_TOKEN"));
}

#[test]
fn serve_refuses_all_interfaces_without_expose() {
    let (_dir, cfg, _) = primed();
    bin()
        .env("MCP_GATEWAY_TOKEN", "fh_mcp_live_dummy")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "serve",
            "petstore",
            "--bind",
            "0.0.0.0:8787",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("--expose"));
}

#[test]
fn compile_hidden_writes_ir() {
    let dir = TempDir::new().unwrap();
    let spec = dir.path().join("tiny.yaml");
    fs::copy("tests/fixtures/tiny.yaml", &spec).unwrap();
    let ir = dir.path().join("ir.json");
    let report = dir.path().join("report.json");
    bin()
        .current_dir(dir.path())
        .args([
            "compile",
            spec.to_str().unwrap(),
            "--out",
            ir.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(ir.exists());
    bin()
        .args(["list-tools", ir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("list_pets"));
}

#[test]
fn version_json() {
    bin()
        .args(["version", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ir\""))
        .stdout(predicate::str::contains("2026-07-28"))
        .stdout(predicate::str::contains("Fetch Hive"));
}

#[test]
fn inspect_client_cursor() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "inspect",
            "petstore",
            "--client",
            "cursor",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\": \"http\""))
        .stdout(predicate::str::contains(".cursor/mcp.json"));
}

#[test]
fn doctor_offline_with_token() {
    let (_dir, cfg, token) = primed();
    let output = bin()
        .env("MCP_GATEWAY_TOKEN", &token)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "doctor",
            "petstore",
            "--offline",
        ])
        .assert()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("ssrf self-test"), "{text}");
    assert!(
        text.contains("\"status\":\"ok\"")
            || text.contains("\"status\": \"ok\"")
            || text.contains("blocked"),
        "{text}"
    );
}

#[test]
fn upgrade_dry_run() {
    bin()
        .args(["upgrade", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry-run"));
}

#[test]
fn serve_stdio_initialize_and_list_tools() {
    let (_dir, cfg, _) = primed();

    let exe = assert_cmd::cargo::cargo_bin("mcp-gateway");
    let mut child = std::process::Command::new(&exe)
        .env_remove("PORT")
        .env_remove("MCP_GATEWAY_SPEC_URL")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "serve",
            "petstore",
            "--stdio",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn stdio server");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        use std::io::Write;
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2026-07-28","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
        )
        .unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    let _ = child.kill();
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("stdio"),
        "banner must be on stderr: {stderr}"
    );
    assert!(
        stdout.contains("list_pets") || stdout.contains("tools"),
        "stdout JSON-RPC should list tools, got {stdout}"
    );
}

#[test]
fn serve_without_token_exits_1() {
    let (_dir, cfg, _) = primed();
    bin()
        .env_remove("MCP_GATEWAY_TOKEN")
        .args(["--config", cfg.to_str().unwrap(), "serve", "petstore"])
        .assert()
        .failure()
        .code(1)
        .stderr(
            predicate::str::contains("MCP_GATEWAY_TOKEN")
                .or(predicate::str::contains("bearer token")),
        );
}

#[test]
fn serve_empty_token_exits_1() {
    let (_dir, cfg, _) = primed();
    bin()
        .env("MCP_GATEWAY_TOKEN", "")
        .args(["--config", cfg.to_str().unwrap(), "serve", "petstore"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("bearer token"));
}

#[test]
fn logs_missing_file_exits_1() {
    let (_dir, cfg, _) = primed();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "logs"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("no log file"));
}

#[test]
fn logs_follow_errors_before_read() {
    let (_dir, cfg, _) = primed();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "logs", "--follow"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("--follow is not implemented"));
}

#[test]
fn auth_remove_after_add() {
    let (_dir, cfg, _) = primed();
    bin()
        .env("PETSTORE_TOKEN", "fh_mcp_live_tmp")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "auth",
            "add",
            "petstore",
            "--type",
            "bearer",
            "--from-env",
            "PETSTORE_TOKEN",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "auth",
            "remove",
            "petstore",
        ])
        .assert()
        .success();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "auth",
            "list",
            "petstore",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("none"));
}

#[test]
fn inspect_tool_prints_http() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "inspect",
            "petstore",
            "--tool",
            "list_pets",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("GET /pets"));
}

#[test]
fn inspect_client_claude() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "inspect",
            "petstore",
            "--client",
            "claude",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"command\": \"mcp-gateway\""));
}

#[test]
fn inspect_client_claude_code() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "inspect",
            "petstore",
            "--client",
            "claude-code",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\": \"http\""))
        .stdout(predicate::str::contains(".mcp.json"));
}

#[test]
fn inspect_client_codex() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "inspect",
            "petstore",
            "--client",
            "codex",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[mcp_servers.petstore]"))
        .stdout(predicate::str::contains("bearer_token_env_var"));
}

#[test]
fn inspect_lists_compiled_tool_names() {
    let (_dir, cfg, _) = primed();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "inspect", "petstore"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list_pets"))
        .stdout(predicate::str::contains("GET"))
        .stdout(predicate::str::contains("/pets"));
}

#[test]
fn inspect_json_includes_tool_names() {
    let (_dir, cfg, _) = primed();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "inspect",
            "petstore",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"tool_names\""))
        .stdout(predicate::str::contains("list_pets"));
}

#[test]
fn inspect_without_name_prints_config() {
    let (_dir, cfg, _) = primed();
    bin()
        .args(["--config", cfg.to_str().unwrap(), "inspect"])
        .assert()
        .success()
        .stdout(predicate::str::contains("config:"));
}

fn spawn_echo() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = b"[]";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    (port, handle)
}

#[test]
fn mcp_gateway_test_hits_loopback_echo() {
    let (_dir, cfg, _) = primed();
    let (port, handle) = spawn_echo();
    bin()
        .env("MCP_GATEWAY_TEST_ALLOW_LOOPBACK", "1")
        .env(
            "MCP_GATEWAY_TEST_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "test",
            "petstore",
            "list_pets",
            "--args",
            "{}",
            "--timeout",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("isError: false"))
        .stdout(predicate::str::contains(format!("http://127.0.0.1:{port}")));
    let _ = handle.join();
}

#[test]
fn test_relative_servers_need_base_url() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success();
    let spec = dir.path().join("rel.yaml");
    fs::copy("tests/fixtures/tiny-relative.yaml", &spec).unwrap();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "add-spec",
            "--name",
            "rel",
            "--file",
            spec.to_str().unwrap(),
        ])
        .assert()
        .success();
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "test",
            "rel",
            "list_pets",
            "--args",
            "{}",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("relative"));
}

#[test]
fn test_base_url_flag_hits_loopback() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success();
    let spec = dir.path().join("rel.yaml");
    fs::copy("tests/fixtures/tiny-relative.yaml", &spec).unwrap();
    let (port, handle) = spawn_echo();
    let origin = format!("http://127.0.0.1:{port}");
    bin()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "add-spec",
            "--name",
            "rel",
            "--file",
            spec.to_str().unwrap(),
            "--base-url",
            &origin,
        ])
        .assert()
        .success();
    bin()
        .env("MCP_GATEWAY_TEST_ALLOW_LOOPBACK", "1")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "test",
            "rel",
            "list_pets",
            "--args",
            "{}",
            "--timeout",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("isError: false"));
    let _ = handle.join();
}

#[test]
fn test_cli_base_url_overrides_without_persisted_spec_field() {
    let (_dir, cfg, _) = primed();
    let (port, handle) = spawn_echo();
    let origin = format!("http://127.0.0.1:{port}");
    bin()
        .env("MCP_GATEWAY_TEST_ALLOW_LOOPBACK", "1")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "test",
            "petstore",
            "list_pets",
            "--args",
            "{}",
            "--timeout",
            "5",
            "--base-url",
            &origin,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("isError: false"));
    let _ = handle.join();
}

#[test]
fn serve_unknown_spec_without_url_fails() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success();
    bin()
        .env("MCP_GATEWAY_TOKEN", "fh_mcp_live_dummy")
        .args(["--config", cfg.to_str().unwrap(), "serve", "demo"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown spec"));
}

#[test]
fn serve_bootstraps_url_through_ssrf() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .env("MCP_GATEWAY_TOKEN", "fh_mcp_live_dummy")
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "serve",
            "demo",
            "--url",
            "https://metadata.google.internal/openapi.json",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("spec URL rejected"));
    assert!(
        !cfg.exists(),
        "failed SSRF must not write a config before compile"
    );
}

#[test]
fn serve_bootstraps_spec_url_env_through_ssrf() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");
    bin()
        .args(["--config", cfg.to_str().unwrap(), "init"])
        .assert()
        .success();
    bin()
        .env("MCP_GATEWAY_TOKEN", "fh_mcp_live_dummy")
        .env(
            "MCP_GATEWAY_SPEC_URL",
            "https://metadata.google.internal/openapi.json",
        )
        .args(["--config", cfg.to_str().unwrap(), "serve", "missing"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("spec URL rejected"));
}

#[test]
fn serve_port_env_listens_all_interfaces() {
    let (_dir, cfg, token) = primed();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let exe = assert_cmd::cargo::cargo_bin("mcp-gateway");
    let mut child = std::process::Command::new(&exe)
        .env("MCP_GATEWAY_TOKEN", &token)
        .env("PORT", port.to_string())
        .env_remove("MCP_GATEWAY_SPEC_URL")
        .args(["--config", cfg.to_str().unwrap(), "serve", "petstore"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut connected = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        connected,
        "serve should honor PORT={port} (config bind is 127.0.0.1:8787)"
    );
}
