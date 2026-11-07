//! Ports of `src/core/bridge/server.test.ts`.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use legate_bridge::{BridgeTool, ToolError, start_bridge};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

use common::{Reply, authed, client, echo_tools, exchange, header, json_body, port_of, rpc};

fn assert_token(token: &str) {
    assert_eq!(token.len(), 48);
    assert!(token.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f')));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn info_describes_a_loopback_url_a_48_char_token_and_the_tool_names() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let url = &bridge.info().url;
    assert!(url.starts_with("http://127.0.0.1:"));
    assert!(url.ends_with("/mcp"));
    assert!(port_of(&bridge) > 0);
    assert_token(&bridge.info().token);
    assert_eq!(bridge.info().tools, ["echo", "boom"]);

    let other = start_bridge(echo_tools()).await.unwrap();
    assert_ne!(bridge.info().token, other.info().token);
    assert_token(&other.info().token);
    other.close().await.unwrap();
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_wrong_and_different_length_tokens_are_401() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let http = client();

    let missing = exchange(&http, reqwest::Method::POST, &bridge.info().url, &[], None).await;
    assert_eq!(missing.status, 401);
    assert_eq!(header(&missing, "www-authenticate"), Some("Bearer"));

    let flipped = flip_first(&bridge.info().token);
    let wrong = authed(
        &http,
        &bridge,
        "{}",
        &[("authorization", &format!("Bearer {flipped}"))],
    )
    .await;
    assert_eq!(wrong.status, 401);
    assert_eq!(header(&wrong, "www-authenticate"), Some("Bearer"));

    let shorter = format!(
        "Bearer {}",
        &bridge.info().token[..bridge.info().token.len() - 1]
    );
    let shorter = authed(&http, &bridge, "{}", &[("authorization", shorter.as_str())]).await;
    assert_eq!(shorter.status, 401);
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_origin_header_is_forbidden() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let port = port_of(&bridge);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let reply = raw_http(
        port,
        &format!("127.0.0.1:{port}"),
        &bridge.info().token,
        body,
        "Origin: http://evil.example\r\n",
    )
    .await;
    assert_eq!(reply.status, 403);
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_must_be_loopback_with_the_bound_port() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let port = port_of(&bridge);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

    let evil = raw_http(port, "evil.example", &bridge.info().token, body, "").await;
    assert_eq!(evil.status, 403);

    let local = raw_http(
        port,
        &format!("localhost:{port}"),
        &bridge.info().token,
        body,
        "",
    )
    .await;
    assert_eq!(local.status, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&local.text).unwrap(),
        json!({"jsonrpc": "2.0", "id": 1, "result": {}})
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_is_not_allowed() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let reply = exchange(
        &client(),
        reqwest::Method::GET,
        &bridge.info().url,
        &[("authorization", &format!("Bearer {}", bridge.info().token))],
        None,
    )
    .await;
    assert_eq!(reply.status, 405);
    assert_eq!(header(&reply, "allow"), Some("POST"));
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_other_than_mcp_is_404() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let url = bridge.info().url.replace("/mcp", "/nope");
    let reply = exchange(
        &client(),
        reqwest::Method::POST,
        &url,
        &[
            ("authorization", &format!("Bearer {}", bridge.info().token)),
            ("content-type", "application/json"),
        ],
        Some("{}".to_string()),
    )
    .await;
    assert_eq!(reply.status, 404);
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_echoes_protocol_version_and_describes_the_server() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let http = client();
    let echoed = rpc(
        &http,
        &bridge,
        json!({"id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}),
    )
    .await;
    assert_eq!(echoed.status, 200);
    assert!(
        header(&echoed, "content-type")
            .unwrap()
            .contains("application/json")
    );
    assert_eq!(
        json_body(&echoed),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "orchestrator", "version": "0.1.0"}
            }
        })
    );

    let fallback = rpc(
        &http,
        &bridge,
        json!({"id": 2, "method": "initialize", "params": {}}),
    )
    .await;
    assert_eq!(result_of(&fallback)["protocolVersion"], json!("2025-06-18"));

    let ignored = rpc(
        &http,
        &bridge,
        json!({"id": 3, "method": "initialize", "params": {"protocolVersion": 5}}),
    )
    .await;
    assert_eq!(result_of(&ignored)["protocolVersion"], json!("2025-06-18"));
    assert_eq!(
        result_of(&ignored)["serverInfo"],
        json!({"name": "orchestrator", "version": "0.1.0"})
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_returns_both_tools_and_their_schemas() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let reply = rpc(&client(), &bridge, json!({"id": 1, "method": "tools/list"})).await;
    assert_eq!(
        result_of(&reply)["tools"],
        json!([
            {
                "name": "echo",
                "description": "Returns a string or an object",
                "inputSchema": {
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "additionalProperties": false
                }
            },
            {
                "name": "boom",
                "description": "Always throws",
                "inputSchema": {"type": "object"}
            }
        ])
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_call_returns_json_text_raw_strings_and_tool_errors() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let http = client();

    let object_result = rpc(
        &http,
        &bridge,
        json!({"id": 1, "method": "tools/call", "params": {"name": "echo"}}),
    )
    .await;
    assert_eq!(
        json_body(&object_result),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"content": [{"type": "text", "text": "{\"ok\":true}"}]}
        })
    );

    let string_result = rpc(
        &http,
        &bridge,
        json!({
            "id": 2,
            "method": "tools/call",
            "params": {"name": "echo", "arguments": {"text": "plain"}}
        }),
    )
    .await;
    assert_eq!(
        result_of(&string_result)["content"],
        json!([{"type": "text", "text": "plain"}])
    );

    let thrown = rpc(
        &http,
        &bridge,
        json!({
            "id": 3,
            "method": "tools/call",
            "params": {"name": "boom", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(
        result_of(&thrown),
        json!({"content": [{"type": "text", "text": "boom"}], "isError": true})
    );

    let unknown = rpc(
        &http,
        &bridge,
        json!({
            "id": 4,
            "method": "tools/call",
            "params": {"name": "missing", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(
        json_body(&unknown),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "error": {"code": -32602, "message": "Unknown tool: missing"}
        })
    );

    let bad_args = rpc(
        &http,
        &bridge,
        json!({
            "id": 5,
            "method": "tools/call",
            "params": {"name": "echo", "arguments": []}
        }),
    )
    .await;
    assert_eq!(json_body(&bad_args)["error"]["code"], json!(-32602));
    assert_eq!(
        json_body(&bad_args)["error"]["message"],
        json!("arguments must be an object")
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_methods_notifications_bad_json_and_batches() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let http = client();

    let unknown = rpc(
        &http,
        &bridge,
        json!({"id": 7, "method": "server/discover"}),
    )
    .await;
    assert_eq!(
        json_body(&unknown),
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "error": {"code": -32601, "message": "Method not found"}
        })
    );

    let note = authed(
        &http,
        &bridge,
        &serde_json::to_string(&json!({"jsonrpc": "2.0", "method": "ping"})).unwrap(),
        &[],
    )
    .await;
    assert_eq!(note.status, 202);
    assert_eq!(note.text, "");

    let bad = authed(&http, &bridge, "{", &[]).await;
    assert_eq!(bad.status, 400);
    assert_eq!(
        serde_json::from_str::<Value>(&bad.text).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32700, "message": "Parse error"}
        })
    );

    let batch = authed(
        &http,
        &bridge,
        &serde_json::to_string(&json!([
            {"jsonrpc": "2.0", "id": 1, "method": "ping"},
            {"jsonrpc": "2.0", "method": "notifications/initialized"}
        ]))
        .unwrap(),
        &[],
    )
    .await;
    assert_eq!(batch.status, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&batch.text).unwrap(),
        json!([{"jsonrpc": "2.0", "id": 1, "result": {}}])
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_over_1_mb_is_413_and_a_body_at_1_mb_is_still_parsed() {
    let bridge = start_bridge(echo_tools()).await.unwrap();
    let http = client();
    let limit = 1024 * 1024;
    let mut at_limit = String::from("{");
    at_limit.push_str(&" ".repeat(limit - 1));
    assert_eq!(at_limit.len(), limit);
    let parsed = authed(&http, &bridge, &at_limit, &[]).await;
    assert_eq!(parsed.status, 400);

    let over = "x".repeat(limit + 1);
    let too_big = authed(&http, &bridge, &over, &[]).await;
    assert_eq!(too_big.status, 413);
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_rejects_a_new_request_resolves_while_one_is_in_flight_and_can_run_twice() {
    let entered = Arc::new(Notify::new());
    let flag = Arc::clone(&entered);
    let hanging = start_bridge(vec![BridgeTool::new(
        "hang",
        "never finishes",
        json!({"type": "object"}),
        move |_args| {
            let flag = Arc::clone(&flag);
            async move {
                flag.notify_one();
                std::future::pending::<Result<Value, ToolError>>().await
            }
        },
    )])
    .await
    .unwrap();

    let http = client();
    let url = hanging.info().url.clone();
    let token = hanging.info().token.clone();
    let pending = tokio::spawn(async move {
        exchange(
            &http,
            reqwest::Method::POST,
            &url,
            &[
                ("authorization", &format!("Bearer {token}")),
                ("content-type", "application/json"),
            ],
            Some(
                serde_json::to_string(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": "hang", "arguments": {}}
                }))
                .unwrap(),
            ),
        )
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .expect("handler entered");
    let started = Instant::now();
    hanging.close().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));

    let finished = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("in-flight request did not end after close");
    assert!(
        finished.is_err(),
        "closed connection should fail the client"
    );

    let again = client().get(&hanging.info().url).send().await;
    assert!(again.is_err(), "bridge should refuse a new connection");
    hanging.close().await.unwrap();
}

fn result_of(reply: &Reply) -> Value {
    json_body(reply)["result"].clone()
}

fn flip_first(token: &str) -> String {
    let mut chars = token.chars();
    let first = chars.next().unwrap();
    let flipped = if first == 'a' { 'b' } else { 'a' };
    format!("{flipped}{}", chars.as_str())
}

struct RawReply {
    status: u16,
    text: String,
}

async fn raw_http(port: u16, host: &str, token: &str, body: &str, extra: &str) -> RawReply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf))
        .await
        .expect("timed out")
        .unwrap();
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").expect("headers");
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    RawReply {
        status,
        text: body.to_string(),
    }
}
