//! Newline-delimited JSON-RPC bridge to one child process.
//!
//! Startup sends one `initialize` with `protocolVersion` `2025-06-18`,
//! `capabilities` `{}`, and `clientInfo` `mcp-gateway` plus this crate's
//! version, then `notifications/initialized`. That `InitializeResult` is
//! cached. Every later `initialize` returns the cache with the caller's own
//! `id` and is not written to the child. A remote `notifications/initialized`
//! is answered with HTTP 202 and is not written to the child.
//!
//! Other requests replace `id` with a monotonic `u64` so two clients can use
//! the same id. The child's `id` is put back before the HTTP response. The
//! wait is 60 seconds, then HTTP 504 `upstream timeout`, and that id is
//! dropped. A late reply for a dropped id is discarded. Startup `initialize`
//! and `tools/list` use the same 60 second wait. A child that does not
//! answer exits the bridge and does not restart. Child stdout lines
//! with both `method` and `id` are server-initiated requests. They are
//! answered on stdin with JSON-RPC `-32601` and `server requests are not
//! bridged`, and stderr gets one warning line. Child notifications (no `id`)
//! are discarded. The first time each method is seen, stderr gets
//! `warning: child notification {method} was discarded`.
//!
//! `GET` is HTTP 405 with an empty body. `DELETE` is HTTP 200 with an empty
//! body. Neither is written to the child. A body over 1048576 bytes is HTTP
//! 413. A JSON array is HTTP 400 `JSON-RPC batches are not accepted`.
//!
//! If the child exits, in-flight requests get HTTP 502 `upstream unreachable`.
//! The process is started again after 200ms, then 400ms, 800ms, 1600ms, and
//! 3200ms. A sixth exit inside the same 60 seconds stops the bridge. The
//! fatal reason is published on [`StdioBridge::fatal`]. stderr from the child
//! is copied to this process with the prefix `stdio: `. A non-JSON stdout
//! line is discarded; the first one prints a warning. The child inherits
//! this process's environment. `MCP_GATEWAY_TOKEN` is removed before spawn.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use http_body_util::{BodyExt, Limited};
use mcp_gateway_tunnel_proto::DEFAULT_MAX_BODY_BYTES;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, watch, RwLock};

use super::{empty_response, json_body, rpc_response};
use crate::LocalService;

const REQUEST_WAIT: Duration = Duration::from_secs(60);
const PROTOCOL: &str = "2025-06-18";
const MAX_EXITS_PER_MINUTE: usize = 5;

enum Job {
    Line {
        line: String,
        original_id: Value,
        child_id: Option<u64>,
        reply: Option<oneshot::Sender<Result<Value, String>>>,
    },
    Cancel {
        child_id: u64,
    },
}

struct Pending {
    original_id: Value,
    reply: oneshot::Sender<Result<Value, String>>,
}

struct Down {
    crashed: bool,
    message: String,
}

#[derive(Clone)]
struct Cache {
    initialize: Value,
}

#[derive(Clone)]
pub struct StdioBridge {
    tx: mpsc::UnboundedSender<Job>,
    cache: Arc<RwLock<Cache>>,
    next_id: Arc<AtomicU64>,
    inflight: Arc<AtomicUsize>,
    wait: Duration,
    fatal: watch::Receiver<Option<String>>,
    report_name: Arc<RwLock<super::ProbeReport>>,
}

impl StdioBridge {
    /// `list_tools` sends one `tools/list` after initialize. `--no-probe`
    /// passes `false`. Initialize still runs, because later initialize calls
    /// are answered from that cache.
    pub async fn start(program: Vec<String>, list_tools: bool) -> Result<Self, String> {
        Self::start_with_wait(program, list_tools, REQUEST_WAIT).await
    }

    /// Same as [`start`](Self::start) with an explicit wait for startup and
    /// each forwarded request. `mcp-gateway tunnel` uses 60 seconds.
    #[doc(hidden)]
    pub async fn start_with_wait(
        program: Vec<String>,
        list_tools: bool,
        wait: Duration,
    ) -> Result<Self, String> {
        if program.is_empty() {
            return Err("stdio command is empty".into());
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let cache = Arc::new(RwLock::new(Cache {
            initialize: Value::Null,
        }));
        let report = Arc::new(RwLock::new(super::ProbeReport {
            name: String::new(),
            version: String::new(),
            tools: 0,
            tools_skipped: !list_tools,
        }));
        let next_id = Arc::new(AtomicU64::new(1));
        let inflight = Arc::new(AtomicUsize::new(0));
        let (fatal_tx, fatal_rx) = watch::channel(None);
        let (ready_tx, ready_rx) = oneshot::channel();
        let cache_task = cache.clone();
        let report_task = report.clone();
        let ids_task = next_id.clone();
        let inflight_task = inflight.clone();
        tokio::spawn(async move {
            supervise(
                program,
                list_tools,
                wait,
                cache_task,
                report_task,
                ids_task,
                inflight_task,
                rx,
                ready_tx,
                fatal_tx,
            )
            .await;
        });
        match ready_rx.await {
            Ok(Ok(())) => Ok(Self {
                tx,
                cache,
                next_id,
                inflight,
                wait,
                fatal: fatal_rx,
                report_name: report,
            }),
            Ok(Err(message)) => Err(message),
            Err(_) => Err("stdio bridge stopped before initialize".into()),
        }
    }

    pub fn fatal(&self) -> watch::Receiver<Option<String>> {
        self.fatal.clone()
    }

    pub async fn report(&self) -> super::ProbeReport {
        self.report_name.read().await.clone()
    }

    #[doc(hidden)]
    pub fn inflight_count(&self) -> usize {
        self.inflight.load(Ordering::SeqCst)
    }
}

impl LocalService for StdioBridge {
    fn call(
        &self,
        request: ::http::Request<Body>,
    ) -> impl Future<Output = ::http::Response<Body>> + Send {
        let tx = self.tx.clone();
        let cache = self.cache.clone();
        let next_id = self.next_id.clone();
        let wait = self.wait;
        async move { handle(tx, cache, next_id, wait, request).await }
    }
}

async fn handle(
    tx: mpsc::UnboundedSender<Job>,
    cache: Arc<RwLock<Cache>>,
    next_id: Arc<AtomicU64>,
    wait: Duration,
    request: ::http::Request<Body>,
) -> ::http::Response<Body> {
    match request.method().as_str() {
        "GET" => return empty_response(405),
        "DELETE" => return empty_response(200),
        "POST" => {}
        _ => return empty_response(405),
    }
    let bytes = match Limited::new(request.into_body(), DEFAULT_MAX_BODY_BYTES as usize)
        .collect()
        .await
    {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return rpc_response(413, -32001, "body exceeds 1048576 bytes", Value::Null);
        }
    };
    if bytes.is_empty() {
        return rpc_response(400, -32700, "expected a JSON-RPC object", Value::Null);
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return rpc_response(400, -32700, "expected a JSON-RPC object", Value::Null),
    };
    if value.is_array() {
        return rpc_response(
            400,
            -32600,
            "JSON-RPC batches are not accepted",
            Value::Null,
        );
    }
    if !value.is_object() {
        return rpc_response(400, -32700, "expected a JSON-RPC object", Value::Null);
    }
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    if method == "notifications/initialized" && value.get("id").is_none() {
        return empty_response(202);
    }
    if method == "initialize" {
        let cached = cache.read().await.initialize.clone();
        if cached.is_null() {
            return rpc_response(502, -32001, "upstream unreachable", Value::Null);
        }
        return json_body(200, &json!({"jsonrpc": "2.0", "id": id, "result": cached}));
    }
    if value.get("id").is_none() {
        let line = match serde_json::to_string(&value) {
            Ok(line) => line,
            Err(_) => return rpc_response(400, -32700, "expected a JSON-RPC object", Value::Null),
        };
        let _ = tx.send(Job::Line {
            line,
            original_id: Value::Null,
            child_id: None,
            reply: None,
        });
        return empty_response(202);
    }

    let child_id = next_id.fetch_add(1, Ordering::SeqCst);
    let (reply_tx, reply_rx) = oneshot::channel();
    let line = json!({
        "jsonrpc": "2.0",
        "id": child_id,
        "method": method,
        "params": value.get("params").cloned().unwrap_or_else(|| json!({}))
    });
    let line = serde_json::to_string(&line).unwrap_or_default();
    if tx
        .send(Job::Line {
            line,
            original_id: id,
            child_id: Some(child_id),
            reply: Some(reply_tx),
        })
        .is_err()
    {
        return rpc_response(502, -32001, "upstream unreachable", Value::Null);
    }
    match tokio::time::timeout(wait, reply_rx).await {
        Ok(Ok(Ok(message))) => json_body(200, &message),
        Ok(Ok(Err(message))) => {
            let status = if message == "upstream timeout" {
                504
            } else {
                502
            };
            rpc_response(status, -32001, &message, Value::Null)
        }
        Ok(Err(_)) => rpc_response(502, -32001, "upstream unreachable", Value::Null),
        Err(_) => {
            let _ = tx.send(Job::Cancel { child_id });
            rpc_response(504, -32001, "upstream timeout", Value::Null)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    program: Vec<String>,
    list_tools: bool,
    wait: Duration,
    cache: Arc<RwLock<Cache>>,
    report: Arc<RwLock<super::ProbeReport>>,
    next_id: Arc<AtomicU64>,
    inflight: Arc<AtomicUsize>,
    mut jobs: mpsc::UnboundedReceiver<Job>,
    ready: oneshot::Sender<Result<(), String>>,
    fatal: watch::Sender<Option<String>>,
) {
    let mut deaths: VecDeque<Instant> = VecDeque::new();
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut ready = Some(ready);
    let mut announced: HashSet<String> = HashSet::new();
    loop {
        if !deaths.is_empty() {
            tokio::time::sleep(backoff(deaths.len())).await;
        }
        let mut child = match spawn_child(&program).await {
            Ok(child) => child,
            Err(message) => {
                finish(&mut ready, &fatal, &mut pending, &inflight, message);
                return;
            }
        };
        match handshake(&mut child, &next_id, wait, list_tools, &mut announced).await {
            Ok((initialize, probed)) => {
                *cache.write().await = Cache { initialize };
                *report.write().await = probed;
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
            }
            Err(err) if err.crashed => {
                fail_pending(&mut pending, &inflight);
                if too_many(&mut deaths) {
                    finish(&mut ready, &fatal, &mut pending, &inflight, err.message);
                    return;
                }
                continue;
            }
            Err(err) => {
                finish(&mut ready, &fatal, &mut pending, &inflight, err.message);
                return;
            }
        }
        match pump(
            &mut child,
            &mut jobs,
            &mut pending,
            &inflight,
            &mut announced,
        )
        .await
        {
            Pump::Exit => {
                fail_pending(&mut pending, &inflight);
                if too_many(&mut deaths) {
                    finish(
                        &mut ready,
                        &fatal,
                        &mut pending,
                        &inflight,
                        format!(
                            "stdio child exited more than {MAX_EXITS_PER_MINUTE} times in 60 seconds"
                        ),
                    );
                    return;
                }
            }
            Pump::Closed => return,
        }
    }
}

enum Pump {
    Exit,
    Closed,
}

struct ChildIo {
    #[allow(dead_code)]
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::UnboundedReceiver<Result<String, String>>,
}

async fn spawn_child(program: &[String]) -> Result<ChildIo, String> {
    let mut command = Command::new(&program[0]);
    command
        .args(&program[1..])
        .env_remove("MCP_GATEWAY_TOKEN")
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start {}: {err}", program[0]))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdio child stdin was not piped".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdio child stdout was not piped".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stdio child stderr was not piped".to_owned())?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("stdio: {line}");
        }
    });
    Ok(ChildIo {
        child,
        stdin,
        lines: spawn_reader(stdout),
    })
}

fn spawn_reader(
    stdout: tokio::process::ChildStdout,
) -> mpsc::UnboundedReceiver<Result<String, String>> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line.is_empty() => continue,
                Ok(Some(line)) => {
                    if tx.send(Ok(line)).is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    let _ = tx.send(Err("stdio child exited".into()));
                    return;
                }
                Err(err) => {
                    let _ = tx.send(Err(format!("stdio child stdout: {err}")));
                    return;
                }
            }
        }
    });
    rx
}

async fn handshake(
    child: &mut ChildIo,
    next_id: &AtomicU64,
    wait: Duration,
    list_tools: bool,
    announced: &mut HashSet<String>,
) -> Result<(Value, super::ProbeReport), Down> {
    let initialize = exchange(
        child,
        next_id,
        wait,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {
                "name": "mcp-gateway",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }),
        announced,
    )
    .await?;
    write_line(
        &mut child.stdin,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
    )
    .await
    .map_err(|message| Down {
        crashed: true,
        message,
    })?;
    let result = initialize.get("result").cloned().ok_or_else(|| Down {
        crashed: false,
        message: "initialize response had no result".into(),
    })?;
    let name = result
        .pointer("/serverInfo/name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let version = result
        .pointer("/serverInfo/version")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let (tools, tools_skipped) = if list_tools {
        let listed = exchange(child, next_id, wait, "tools/list", json!({}), announced).await?;
        let count = listed
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| Down {
                crashed: false,
                message: "tools/list did not return a tools array".into(),
            })?;
        (count, false)
    } else {
        (0, true)
    };
    Ok((
        result,
        super::ProbeReport {
            name,
            version,
            tools,
            tools_skipped,
        },
    ))
}

async fn exchange(
    child: &mut ChildIo,
    next_id: &AtomicU64,
    wait: Duration,
    method: &str,
    params: Value,
    announced: &mut HashSet<String>,
) -> Result<Value, Down> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let line = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string();
    write_line(&mut child.stdin, &line)
        .await
        .map_err(|message| Down {
            crashed: true,
            message,
        })?;
    loop {
        let line = match tokio::time::timeout(wait, next_stdout(&mut child.lines)).await {
            Ok(line) => line?,
            Err(_) => {
                return Err(Down {
                    crashed: false,
                    message: format!("{method} timed out after {}", wait_label(wait)),
                });
            }
        };
        match classify(&line) {
            Line::Request(value) => reject_child(&mut child.stdin, &value).await?,
            Line::Notification(value) => note_notification(&value, announced),
            Line::Response(value) => {
                if value.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(Down {
                        crashed: false,
                        message: format!("{method} failed: {error}"),
                    });
                }
                return Ok(value);
            }
            Line::Ignore => {}
        }
    }
}

async fn pump(
    child: &mut ChildIo,
    jobs: &mut mpsc::UnboundedReceiver<Job>,
    pending: &mut HashMap<u64, Pending>,
    inflight: &AtomicUsize,
    announced: &mut HashSet<String>,
) -> Pump {
    loop {
        tokio::select! {
            job = jobs.recv() => {
                let Some(job) = job else {
                    return Pump::Closed;
                };
                match job {
                    Job::Cancel { child_id } => {
                        drop_pending(pending, inflight, child_id);
                    }
                    Job::Line { line, original_id, child_id, reply } => {
                        if let (Some(child_id), Some(reply)) = (child_id, reply) {
                            pending.insert(child_id, Pending { original_id, reply });
                            inflight.fetch_add(1, Ordering::SeqCst);
                        }
                        if write_line(&mut child.stdin, &line).await.is_err() {
                            return Pump::Exit;
                        }
                    }
                }
            }
            line = next_stdout(&mut child.lines) => {
                let line = match line {
                    Ok(line) => line,
                    Err(_) => return Pump::Exit,
                };
                match classify(&line) {
                    Line::Request(value) => {
                        if reject_child(&mut child.stdin, &value).await.is_err() {
                            return Pump::Exit;
                        }
                    }
                    Line::Notification(value) => note_notification(&value, announced),
                    Line::Response(value) => deliver(pending, inflight, value),
                    Line::Ignore => {}
                }
            }
        }
    }
}

fn drop_pending(pending: &mut HashMap<u64, Pending>, inflight: &AtomicUsize, id: u64) {
    if pending.remove(&id).is_some() {
        inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn deliver(pending: &mut HashMap<u64, Pending>, inflight: &AtomicUsize, mut value: Value) {
    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        return;
    };
    let Some(pending) = pending.remove(&id) else {
        return;
    };
    inflight.fetch_sub(1, Ordering::SeqCst);
    value["id"] = pending.original_id;
    let _ = pending.reply.send(Ok(value));
}

async fn reject_child(stdin: &mut ChildStdin, value: &Value) -> Result<(), Down> {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("request");
    eprintln!("warning: child request {method} is not bridged; answered with JSON-RPC -32601");
    let line = json!({
        "jsonrpc": "2.0",
        "id": value.get("id").cloned().unwrap_or(Value::Null),
        "error": { "code": -32601, "message": "server requests are not bridged" }
    });
    write_line(stdin, &line.to_string())
        .await
        .map_err(|message| Down {
            crashed: true,
            message,
        })
}

fn note_notification(value: &Value, announced: &mut HashSet<String>) {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    if announced.insert(method.to_owned()) {
        eprintln!("warning: child notification {method} was discarded");
    }
}

enum Line {
    Request(Value),
    Response(Value),
    Notification(Value),
    Ignore,
}

fn classify(line: &str) -> Line {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::SeqCst) {
            eprintln!(
                "warning: stdio child wrote a non-JSON line; further non-JSON lines are discarded"
            );
        }
        return Line::Ignore;
    };
    let has_method = value.get("method").and_then(Value::as_str).is_some();
    let has_id = value.get("id").is_some();
    if has_method && has_id {
        Line::Request(value)
    } else if has_id {
        Line::Response(value)
    } else if has_method {
        Line::Notification(value)
    } else {
        Line::Ignore
    }
}

async fn next_stdout(
    lines: &mut mpsc::UnboundedReceiver<Result<String, String>>,
) -> Result<String, Down> {
    match lines.recv().await {
        Some(Ok(line)) => Ok(line),
        Some(Err(message)) => Err(Down {
            crashed: true,
            message,
        }),
        None => Err(Down {
            crashed: true,
            message: "stdio child exited".into(),
        }),
    }
}

async fn write_line(stdin: &mut ChildStdin, line: &str) -> Result<(), String> {
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|err| err.to_string())?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|err| err.to_string())?;
    stdin.flush().await.map_err(|err| err.to_string())?;
    Ok(())
}

fn fail_pending(pending: &mut HashMap<u64, Pending>, inflight: &AtomicUsize) {
    for (_, pending) in pending.drain() {
        let _ = pending.reply.send(Err("upstream unreachable".into()));
    }
    inflight.store(0, Ordering::SeqCst);
}

fn finish(
    ready: &mut Option<oneshot::Sender<Result<(), String>>>,
    fatal: &watch::Sender<Option<String>>,
    pending: &mut HashMap<u64, Pending>,
    inflight: &AtomicUsize,
    message: String,
) {
    fail_pending(pending, inflight);
    if let Some(ready) = ready.take() {
        let _ = ready.send(Err(message.clone()));
    }
    let _ = fatal.send(Some(message));
}

fn too_many(deaths: &mut VecDeque<Instant>) -> bool {
    let now = Instant::now();
    while deaths
        .front()
        .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
    {
        deaths.pop_front();
    }
    deaths.push_back(now);
    deaths.len() > MAX_EXITS_PER_MINUTE
}

fn wait_label(wait: Duration) -> String {
    let ms = wait.as_millis();
    if ms >= 1000 && ms.is_multiple_of(1000) {
        format!("{} seconds", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

fn backoff(deaths: usize) -> Duration {
    let shift = u32::try_from(deaths.saturating_sub(1)).unwrap_or(4).min(4);
    Duration::from_millis(200u64.saturating_mul(1u64 << shift))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_200ms_and_caps() {
        assert_eq!(backoff(1), Duration::from_millis(200));
        assert_eq!(backoff(2), Duration::from_millis(400));
        assert_eq!(backoff(5), Duration::from_millis(3200));
        assert_eq!(backoff(9), Duration::from_millis(3200));
    }

    #[test]
    fn sixth_exit_inside_the_window_is_fatal() {
        let mut deaths = VecDeque::new();
        for _ in 0..5 {
            assert!(!too_many(&mut deaths));
        }
        assert!(too_many(&mut deaths));
    }

    #[tokio::test]
    async fn handshake_timeout_does_not_retry() {
        let started = StdioBridge::start_with_wait(
            vec!["sleep".into(), "30".into()],
            false,
            Duration::from_millis(200),
        )
        .await;
        match started {
            Err(err) => assert!(err.contains("timed out"), "{err}"),
            Ok(_) => panic!("sleep should not finish initialize"),
        }
    }
}
