//! JSON-RPC over HTTP, matching `src/core/bridge/server.ts`.
//!
//! `POST /mcp` only. No server-sent events and no session header. One bearer
//! token per [`start_bridge`], compared in constant time when the lengths match.

use std::convert::Infallible;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use legate_core::BridgeInfo;
use serde::Serialize;
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tower::ServiceExt;

use crate::error::{BridgeError, ToolError};

/// Largest request body, in bytes. Anything larger is refused before it is parsed.
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// How long an error response waits for the client to finish uploading.
const DRAIN: Duration = Duration::from_millis(2_000);
/// Cap on bytes discarded while draining an error response.
const DRAIN_MAX_BYTES: usize = 8 * 1024 * 1024;

const PARSE_ERROR: &str =
    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Handler = Arc<dyn Fn(Map<String, Value>) -> BoxFuture<Result<Value, ToolError>> + Send + Sync>;

enum InputSchema {
    Raw(&'static str),
    Value(Value),
}

/// One MCP tool the bridge can call.
pub struct BridgeTool {
    /// Tool name, as `tools/list` and `BridgeInfo.tools` report it.
    pub name: String,
    /// Tool description. Built-in descriptions match `tools.ts` byte for byte.
    pub description: String,
    schema: InputSchema,
    handler: Handler,
}

impl BridgeTool {
    /// A tool whose schema is a JSON value.
    pub fn new<F, Fut>(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        handler: F,
    ) -> Self
    where
        F: Fn(Map<String, Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, ToolError>> + Send + 'static,
    {
        Self::from_parts(
            name.into(),
            description.into(),
            InputSchema::Value(input_schema),
            Arc::new(move |args| Box::pin(handler(args))),
        )
    }

    pub(crate) fn from_raw<F, Fut>(
        name: &'static str,
        description: &'static str,
        schema: &'static str,
        handler: F,
    ) -> Self
    where
        F: Fn(Map<String, Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, ToolError>> + Send + 'static,
    {
        Self::from_parts(
            name.to_string(),
            description.to_string(),
            InputSchema::Raw(schema),
            Arc::new(move |args| Box::pin(handler(args))),
        )
    }

    fn from_parts(
        name: String,
        description: String,
        schema: InputSchema,
        handler: Handler,
    ) -> Self {
        Self {
            name,
            description,
            schema,
            handler,
        }
    }
}

struct AppState {
    tools: Vec<BridgeTool>,
    token: String,
    port: u16,
}

/// A bridge bound to `127.0.0.1` on an ephemeral port.
///
/// Dropping it aborts the accept loop. [`Bridge::close`] waits until current
/// connections are dropped, which is what a hanging tool call needs.
#[must_use = "the server runs until close or drop"]
pub struct Bridge {
    info: BridgeInfo,
    shutdown: watch::Sender<bool>,
    finished: watch::Sender<bool>,
    /// Keeps the finished-watch open so `send` still updates the value.
    _finished_rx: watch::Receiver<bool>,
    leader: Mutex<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Bridge {
    /// URL, bearer token, and tool names.
    pub fn info(&self) -> &BridgeInfo {
        &self.info
    }

    /// Stop accepting, drop open connections, and wait for the accept loop.
    ///
    /// A second call waits for the first and then returns. Safe to call after
    /// the server has already stopped.
    pub async fn close(&self) -> Result<(), BridgeError> {
        let leader = {
            let mut leader = lock(&self.leader);
            if *self.finished.subscribe().borrow() {
                return Ok(());
            }
            if *leader {
                false
            } else {
                *leader = true;
                true
            }
        };
        if !leader {
            return self.wait_finished().await;
        }
        let _finish = Finish(self.finished.clone());
        let _ = self.shutdown.send(true);
        let task = lock(&self.task).take();
        if let Some(task) = task {
            match task.await {
                Ok(()) => {}
                Err(err) if err.is_cancelled() => {}
                Err(err) => {
                    tracing::error!("bridge server panicked: {err}");
                    return Err(BridgeError::Stopped);
                }
            }
        }
        Ok(())
    }

    async fn wait_finished(&self) -> Result<(), BridgeError> {
        let mut rx = self.finished.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return Ok(());
            }
            if rx.changed().await.is_err() {
                return Ok(());
            }
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
    }
}

struct Finish(watch::Sender<bool>);

impl Drop for Finish {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

/// Bind `127.0.0.1:0`, generate a token, and serve `tools`.
///
/// The returned [`Bridge`] is [`Send`]. Its accept loop is spawned on the
/// current Tokio runtime, which must be a multi-thread runtime in the app.
pub async fn start_bridge(tools: Vec<BridgeTool>) -> Result<Bridge, BridgeError> {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Bridge>();
    assert_send_sync::<BridgeTool>();

    let token = random_token()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(BridgeError::Bind)?;
    let port = listener.local_addr().map_err(BridgeError::Address)?.port();
    let names = tools.iter().map(|tool| tool.name.clone()).collect();
    let state = Arc::new(AppState {
        tools,
        token: token.clone(),
        port,
    });
    let app = Router::new().fallback(handle).with_state(state);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (finished_tx, finished_rx) = watch::channel(false);
    let task = tokio::spawn(run(listener, app, shutdown_rx));
    Ok(Bridge {
        info: BridgeInfo {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token,
            tools: names,
        },
        shutdown: shutdown_tx,
        finished: finished_tx,
        _finished_rx: finished_rx,
        leader: Mutex::new(false),
        task: Mutex::new(Some(task)),
    })
}

async fn run(listener: TcpListener, app: Router, mut shutdown: watch::Receiver<bool>) {
    let mut joins = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((socket, _)) => {
                        let app = app.clone();
                        joins.spawn(async move {
                            serve_socket(socket, app).await;
                        });
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        tracing::error!("bridge accept failed: {err}");
                        break;
                    }
                }
            }
            _ = joins.join_next(), if !joins.is_empty() => {}
        }
    }
    joins.abort_all();
    while joins.join_next().await.is_some() {}
}

async fn serve_socket(socket: tokio::net::TcpStream, app: Router) {
    let io = TokioIo::new(socket);
    let service = hyper::service::service_fn(move |request: hyper::Request<Incoming>| {
        let app = app.clone();
        async move {
            let (parts, body) = request.into_parts();
            let request = Request::from_parts(parts, Body::new(body));
            let response = match app.oneshot(request).await {
                Ok(response) => response,
                Err(never) => match never {},
            };
            Ok::<Response, Infallible>(response)
        }
    });
    let connection = hyper::server::conn::http1::Builder::new().serve_connection(io, service);
    if let Err(err) = connection.await {
        tracing::debug!("bridge connection ended: {err}");
    }
}

async fn handle(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    if request_path(&parts.uri) != "/mcp" {
        return reject(body, StatusCode::NOT_FOUND, &[]).await;
    }
    if parts.headers.contains_key(axum::http::header::ORIGIN) {
        return reject(body, StatusCode::FORBIDDEN, &[]).await;
    }
    if !allowed_host(parts.headers.get(axum::http::header::HOST), state.port) {
        return reject(body, StatusCode::FORBIDDEN, &[]).await;
    }
    if !authorized(
        parts.headers.get(axum::http::header::AUTHORIZATION),
        &state.token,
    ) {
        return reject(
            body,
            StatusCode::UNAUTHORIZED,
            &[("www-authenticate", "Bearer")],
        )
        .await;
    }
    if parts.method != axum::http::Method::POST {
        return reject(body, StatusCode::METHOD_NOT_ALLOWED, &[("allow", "POST")]).await;
    }
    if declared_length(&parts.headers).is_some_and(|len| len > MAX_BODY_BYTES as u64) {
        return reject(body, StatusCode::PAYLOAD_TOO_LARGE, &[]).await;
    }
    let bytes = match read_limited(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(ReadError::TooLarge) => {
            return empty_response(StatusCode::PAYLOAD_TOO_LARGE, true, &[]);
        }
        Err(ReadError::Failed) => {
            tracing::error!("bridge failed to read the request body");
            return empty_response(StatusCode::INTERNAL_SERVER_ERROR, false, &[]);
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return json_text(StatusCode::BAD_REQUEST, PARSE_ERROR),
    };
    let parsed = match serde_json::from_str::<Value>(&text) {
        Ok(value) => value,
        Err(_) => return json_text(StatusCode::BAD_REQUEST, PARSE_ERROR),
    };
    match respond(&parsed, &state.tools).await {
        Ok(Respond::Empty) => empty_response(StatusCode::ACCEPTED, false, &[]),
        Ok(Respond::Json(body)) => json_text(StatusCode::OK, &body),
        Err(message) => {
            tracing::error!("bridge request failed: {message}");
            empty_response(StatusCode::INTERNAL_SERVER_ERROR, false, &[])
        }
    }
}

enum Respond {
    Empty,
    Json(String),
}

async fn respond(parsed: &Value, tools: &[BridgeTool]) -> Result<Respond, String> {
    if let Some(items) = parsed.as_array() {
        let mut responses = Vec::new();
        for item in items {
            if let Some(body) = one_response(item, tools).await? {
                responses.push(body);
            }
        }
        if responses.is_empty() {
            return Ok(Respond::Empty);
        }
        return Ok(Respond::Json(join_array(&responses)));
    }
    match one_response(parsed, tools).await? {
        Some(body) => Ok(Respond::Json(body)),
        None => Ok(Respond::Empty),
    }
}

async fn one_response(message: &Value, tools: &[BridgeTool]) -> Result<Option<String>, String> {
    let Some(record) = message.as_object() else {
        return Ok(Some(rpc_error(&Value::Null, -32600, "Invalid Request")?));
    };
    let has_id = record.contains_key("id");
    let id = id_value(record.get("id"));
    let Some(method) = record.get("method").and_then(Value::as_str) else {
        return if has_id {
            Ok(Some(rpc_error(&id, -32600, "Invalid Request")?))
        } else {
            Ok(None)
        };
    };
    if !has_id {
        return Ok(None);
    }
    match invoke(&id, method, record.get("params"), tools).await {
        Ok(body) => Ok(Some(body)),
        Err(InvokeError::Protocol { code, message }) => Ok(Some(rpc_error(&id, code, &message)?)),
        Err(InvokeError::Internal(message)) => Err(message),
    }
}

enum InvokeError {
    Protocol { code: i32, message: String },
    Internal(String),
}

async fn invoke(
    id: &Value,
    method: &str,
    params: Option<&Value>,
    tools: &[BridgeTool],
) -> Result<String, InvokeError> {
    match method {
        "initialize" => rpc_result(id, &initialize_result(params)).map_err(internal),
        "ping" => rpc_result(id, &Empty {}).map_err(internal),
        "tools/list" => rpc_result(id, &tools_list(tools)).map_err(internal),
        "tools/call" => {
            let body = call_tool(params, tools).await?;
            rpc_result(id, &body).map_err(internal)
        }
        _ => Err(InvokeError::Protocol {
            code: -32601,
            message: "Method not found".to_string(),
        }),
    }
}

fn initialize_result(params: Option<&Value>) -> InitializeResult {
    let protocol_version = params
        .and_then(Value::as_object)
        .and_then(|map| map.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or("2025-06-18");
    InitializeResult {
        protocol_version: protocol_version.to_string(),
        capabilities: Capabilities { tools: Empty {} },
        server_info: ServerInfo {
            name: "orchestrator",
            version: "0.1.0",
        },
    }
}

fn tools_list(tools: &[BridgeTool]) -> ToolsList<'_> {
    ToolsList {
        tools: tools
            .iter()
            .map(|tool| ListedTool {
                name: &tool.name,
                description: &tool.description,
                input_schema: SchemaSer(&tool.schema),
            })
            .collect(),
    }
}

async fn call_tool(params: Option<&Value>, tools: &[BridgeTool]) -> Result<ToolBody, InvokeError> {
    let record = params.and_then(Value::as_object);
    let name = match record.and_then(|map| map.get("name")) {
        Some(Value::String(name)) => name.as_str(),
        Some(other) => {
            return Err(InvokeError::Protocol {
                code: -32602,
                message: format!("Unknown tool: {}", js_string(other)),
            });
        }
        None => {
            return Err(InvokeError::Protocol {
                code: -32602,
                message: "Unknown tool: undefined".to_string(),
            });
        }
    };
    let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
        return Err(InvokeError::Protocol {
            code: -32602,
            message: format!("Unknown tool: {name}"),
        });
    };
    let args = match record.and_then(|map| map.get("arguments")) {
        Some(Value::Object(map)) => map.clone(),
        Some(_) => {
            return Err(InvokeError::Protocol {
                code: -32602,
                message: "arguments must be an object".to_string(),
            });
        }
        None => Map::new(),
    };
    let outcome = (tool.handler)(args).await;
    let body = match outcome {
        Ok(value) => ToolBody {
            content: [Content {
                kind: "text",
                text: text_of(&value),
            }],
            is_error: None,
        },
        Err(err) => ToolBody {
            content: [Content {
                kind: "text",
                text: err.message().to_string(),
            }],
            is_error: Some(true),
        },
    };
    Ok(body)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn id_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Number(number)) => Value::Number(number.clone()),
        Some(Value::Null) | None => Value::Null,
        Some(_) => Value::Null,
    }
}

fn rpc_result(id: &Value, result: &impl Serialize) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Body<'a, T: Serialize> {
        jsonrpc: &'static str,
        id: &'a Value,
        result: &'a T,
    }
    serde_json::to_string(&Body {
        jsonrpc: "2.0",
        id,
        result,
    })
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Result<String, String> {
    #[derive(Serialize)]
    struct Body<'a> {
        jsonrpc: &'static str,
        id: &'a Value,
        error: ErrorBody<'a>,
    }
    #[derive(Serialize)]
    struct ErrorBody<'a> {
        code: i32,
        message: &'a str,
    }
    serde_json::to_string(&Body {
        jsonrpc: "2.0",
        id,
        error: ErrorBody { code, message },
    })
    .map_err(|err| err.to_string())
}

fn internal(err: serde_json::Error) -> InvokeError {
    InvokeError::Internal(err.to_string())
}

fn join_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(item);
    }
    out.push(']');
    out
}

fn request_path(uri: &axum::http::Uri) -> &str {
    uri.path()
}

fn allowed_host(host: Option<&HeaderValue>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.as_bytes();
    host == format!("127.0.0.1:{port}").as_bytes() || host == format!("localhost:{port}").as_bytes()
}

fn authorized(header: Option<&HeaderValue>, token: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let expected = format!("Bearer {token}");
    let got = header.as_bytes();
    let want = expected.as_bytes();
    if got.len() != want.len() {
        return false;
    }
    got.ct_eq(want).into()
}

/// `Content-Length` when it is a non-negative integer, matching the TypeScript
/// guard. Anything else is left for the streaming limit.
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(axum::http::header::CONTENT_LENGTH)?;
    let text = value.to_str().ok()?.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

enum ReadError {
    TooLarge,
    Failed,
}

async fn read_limited(mut body: Body, limit: usize) -> Result<Vec<u8>, ReadError> {
    let mut buf = Vec::new();
    loop {
        match body.frame().await {
            Some(Ok(frame)) => {
                let Ok(data) = frame.into_data() else {
                    continue;
                };
                if buf.len().saturating_add(data.len()) > limit {
                    return Err(ReadError::TooLarge);
                }
                buf.extend_from_slice(&data);
            }
            Some(Err(_)) => return Err(ReadError::Failed),
            None => return Ok(buf),
        }
    }
}

/// Read the upload off the socket before answering. Closing while the client
/// is still sending resets the connection on Windows, and the client then
/// never sees the status. Bytes are not kept. A sender that will not stop is
/// cut off by size or by time.
async fn drain(mut body: Body) {
    let sleep = tokio::time::sleep(DRAIN);
    tokio::pin!(sleep);
    let mut drained = 0usize;
    loop {
        tokio::select! {
            () = &mut sleep => break,
            frame = body.frame() => {
                match frame {
                    Some(Ok(frame)) => {
                        let Ok(data) = frame.into_data() else {
                            continue;
                        };
                        drained = drained.saturating_add(data.len());
                        if drained > DRAIN_MAX_BYTES {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
    }
}

async fn reject(body: Body, status: StatusCode, extra: &[(&str, &str)]) -> Response {
    drain(body).await;
    empty_response(status, true, extra)
}

fn empty_response(status: StatusCode, close: bool, extra: &[(&str, &str)]) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    let headers = response.headers_mut();
    insert_header(headers, "content-length", "0");
    if close {
        insert_header(headers, "connection", "close");
    }
    for (name, value) in extra {
        insert_header(headers, name, value);
    }
    response
}

fn json_text(status: StatusCode, body: &str) -> Response {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    insert_header(headers, "content-type", "application/json");
    insert_header(headers, "content-length", &body.len().to_string());
    response
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    let Ok(name) = HeaderName::try_from(name) else {
        return;
    };
    let Ok(value) = HeaderValue::try_from(value) else {
        return;
    };
    headers.insert(name, value);
}

fn random_token() -> Result<String, BridgeError> {
    let mut bytes = [0u8; 24];
    getrandom::getrandom(&mut bytes).map_err(BridgeError::Token)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(48);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[derive(Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Capabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

#[derive(Serialize)]
struct Capabilities {
    tools: Empty,
}

#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
struct ServerInfo {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct ToolsList<'a> {
    tools: Vec<ListedTool<'a>>,
}

#[derive(Serialize)]
struct ListedTool<'a> {
    name: &'a str,
    description: &'a str,
    #[serde(rename = "inputSchema")]
    input_schema: SchemaSer<'a>,
}

struct SchemaSer<'a>(&'a InputSchema);

impl Serialize for SchemaSer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.0 {
            InputSchema::Raw(raw) => {
                let raw = serde_json::value::RawValue::from_string((*raw).to_string())
                    .map_err(serde::ser::Error::custom)?;
                raw.serialize(serializer)
            }
            InputSchema::Value(value) => value.serialize(serializer),
        }
    }
}

#[derive(Serialize)]
struct ToolBody {
    content: [Content; 1],
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Serialize)]
struct Content {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}
