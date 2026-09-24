use axum::body::Body;
use http_body_util::BodyExt;
use mcp_gateway_tunnel::upstream::StdioBridge;
use mcp_gateway_tunnel::LocalService;
use serde_json::{json, Value};

fn bin() -> String {
    env!("CARGO_BIN_EXE_stdio_fixture").to_owned()
}

fn post(body: Value) -> http::Request<Body> {
    http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn call(bridge: &StdioBridge, body: Value) -> (u16, Value) {
    let response = bridge.call(post(body)).await;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn cached_initialize_is_shared_and_ids_are_remapped() {
    let bridge = StdioBridge::start(vec![bin()], false).await.unwrap();
    let mut joins = tokio::task::JoinSet::new();
    for id in 0..3 {
        let bridge = bridge.clone();
        joins.spawn(async move {
            call(
                &bridge,
                json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{}}),
            )
            .await
        });
    }
    let mut ids = Vec::new();
    while let Some(joined) = joins.join_next().await {
        let (status, value) = joined.unwrap();
        assert_eq!(status, 200);
        assert_eq!(value["result"]["inits"], 1);
        assert_eq!(value["result"]["serverInfo"]["name"], "fixture");
        ids.push(value["id"].clone());
    }
    ids.sort_by_key(|id| id.as_u64().unwrap());
    assert_eq!(ids, vec![json!(0), json!(1), json!(2)]);

    let bridge_a = bridge.clone();
    let bridge_b = bridge.clone();
    let (left, right) = tokio::join!(
        call(
            &bridge_a,
            json!({"jsonrpc":"2.0","id":"same","method":"tools/list","params":{}})
        ),
        call(
            &bridge_b,
            json!({"jsonrpc":"2.0","id":"same","method":"tools/list","params":{}})
        ),
    );
    assert_eq!(left.0, 200);
    assert_eq!(right.0, 200);
    assert_eq!(left.1["id"], "same");
    assert_eq!(right.1["id"], "same");
    assert_ne!(left.1["result"]["child_id"], right.1["result"]["child_id"]);
}

#[tokio::test]
async fn get_and_delete_do_not_reach_the_child() {
    let bridge = StdioBridge::start(vec![bin()], false).await.unwrap();
    let get = bridge
        .call(
            http::Request::builder()
                .method("GET")
                .uri("/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get.status(), 405);
    let delete = bridge
        .call(
            http::Request::builder()
                .method("DELETE")
                .uri("/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(delete.status(), 200);
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(value["result"]["inits"], 1);
    assert_eq!(value["result"]["lines"], 3);
}

#[tokio::test]
async fn child_exit_fails_the_inflight_call_and_the_next_call_works() {
    let bridge = StdioBridge::start(vec![bin()], false).await.unwrap();
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":1,"method":"exit","params":{}}),
    )
    .await;
    assert_eq!(status, 502);
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("upstream unreachable"));
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    // A new process starts at zero. Its startup initialize is the only one it saw.
    assert_eq!(value["result"]["inits"], 1);
    assert_eq!(value["result"]["lines"], 3);
}

#[tokio::test]
async fn child_request_is_rejected_with_method_not_found() {
    let bridge = StdioBridge::start(vec![bin(), "--request".into()], false)
        .await
        .unwrap();
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["result"]["child_rejected"], true);
}

#[tokio::test]
async fn timed_out_request_is_dropped_and_the_next_call_works() {
    let bridge =
        StdioBridge::start_with_wait(vec![bin()], false, std::time::Duration::from_secs(2))
            .await
            .unwrap();
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":7,"method":"hang","params":{}}),
    )
    .await;
    assert_eq!(status, 504);
    assert_eq!(value["error"]["message"], "upstream timeout");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(bridge.inflight_count(), 0);
    let (status, listed) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":8,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(listed["id"], 8);
}

#[tokio::test]
async fn child_does_not_inherit_the_gateway_token() {
    unsafe { std::env::set_var("MCP_GATEWAY_TOKEN", "super-secret") };
    let bridge = StdioBridge::start(vec![bin()], true).await.unwrap();
    unsafe { std::env::remove_var("MCP_GATEWAY_TOKEN") };
    let (status, value) = call(
        &bridge,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(value["result"]["gateway_token_present"], false);
}

#[tokio::test]
async fn batches_are_refused() {
    let bridge = StdioBridge::start(vec![bin()], false).await.unwrap();
    let (status, value) = call(&bridge, json!([])).await;
    assert_eq!(status, 400);
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("batches"));
}
