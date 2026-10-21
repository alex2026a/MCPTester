//! 遗留 HTTP+SSE 传输（自研；规范已弃用，用于兼容旧服务器）
//!
//! 流程（2024-11-05 传输约定 + 官方 TS SDK 实现约定）：
//! 1. `GET {url}`（`Accept: text/event-stream`）建立事件流；
//! 2. 服务器以 `event: endpoint` 事件返回消息地址（相对地址基于原 URL 解析）；
//! 3. 客户端消息以 `POST {endpoint}` 发送（期待 202/200）；
//! 4. 服务端消息以 `message` 事件承载 JSON-RPC。
//!
//! 说明：`Last-Event-ID` 续传、202 语义、会话失效码均为实现约定而非规范；
//! 约定可缺失（无 Last-Event-ID 时直接重连），与 02 文档 3.3 一致。
//! 本模块通过 `(Sink, Stream)` 通道对接入 rmcp（`IntoTransport`）。

use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use rmcp::RoleClient;
use rmcp::model::ServerJsonRpcMessage;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use tokio::sync::watch;
use url::Url;

use crate::mcp::error::McpError;
use crate::mcp::handler::EventHub;
use crate::mcp::models::HttpConfig;

/// SSE 传输组件：通道对 + 工作线程句柄（断开时中止）
pub struct SseTransport {
    pub sink: mpsc::Sender<TxJsonRpcMessage<RoleClient>>,
    pub stream: mpsc::Receiver<RxJsonRpcMessage<RoleClient>>,
    pub workers: Vec<tokio::task::JoinHandle<()>>,
}

/// 构建 SSE 传输：启动事件流读取与消息发送两个工作线程
pub fn build_sse(
    cfg: &HttpConfig,
    default_timeout_secs: u64,
    hub: &Arc<EventHub>,
) -> Result<SseTransport, McpError> {
    let base = Url::parse(&cfg.url).map_err(|e| McpError::Config {
        message: format!("URL 非法：{e}"),
    })?;
    if base.scheme() != "http" && base.scheme() != "https" {
        return Err(McpError::Config {
            message: "SSE 传输仅支持 http/https URL".to_string(),
        });
    }

    let mut headers = reqwest::header::HeaderMap::new();
    for kv in &cfg.headers {
        let Some(value) = kv.value.as_deref() else {
            continue;
        };
        let name = reqwest::header::HeaderName::from_bytes(kv.key.as_bytes()).map_err(|e| {
            McpError::Config {
                message: format!("请求头名称非法「{}」：{e}", kv.key),
            }
        })?;

        // v0.1.1：与 Streamable HTTP 统一归一化（完整 Bearer / 纯 token / 其他 scheme 均可）
        if name == reqwest::header::AUTHORIZATION {
            if let Some(normalized) = super::normalize_authorization(value) {
                let v =
                    reqwest::header::HeaderValue::from_str(&normalized.header_value()).map_err(
                        |e| McpError::Config {
                            message: format!("Authorization 头值非法：{e}"),
                        },
                    )?;
                headers.insert(name, v);
            }
            continue;
        }

        let value = reqwest::header::HeaderValue::from_str(value).map_err(|e| McpError::Config {
            message: format!("请求头值非法「{}」：{e}", kv.key),
        })?;
        headers.insert(name, value);
    }

    let timeout_secs = cfg.timeout_secs.unwrap_or(default_timeout_secs).max(1);
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(timeout_secs * 2));
    if cfg.allow_insecure_tls {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().map_err(|e| McpError::TransportFailed {
        message: format!("HTTP 客户端创建失败：{e}"),
    })?;

    let (out_tx, out_rx) = mpsc::channel::<TxJsonRpcMessage<RoleClient>>(64);
    let (in_tx, in_rx) = mpsc::channel::<RxJsonRpcMessage<RoleClient>>(64);
    let (endpoint_tx, endpoint_rx) = watch::channel::<Option<Url>>(None);

    let reader = spawn_reader(
        client.clone(),
        base,
        headers,
        in_tx,
        endpoint_tx,
        Arc::clone(hub),
    );
    let sender = spawn_sender(client, out_rx, endpoint_rx, Arc::clone(hub));

    Ok(SseTransport {
        sink: out_tx,
        stream: in_rx,
        workers: vec![reader, sender],
    })
}

/// 读取事件流：解析 endpoint 与 message 事件
fn spawn_reader(
    client: reqwest::Client,
    base: Url,
    headers: reqwest::header::HeaderMap,
    mut in_tx: mpsc::Sender<RxJsonRpcMessage<RoleClient>>,
    endpoint_tx: watch::Sender<Option<Url>>,
    hub: Arc<EventHub>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let request = client
            .get(base.clone())
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .headers(headers);
        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                hub.push_notification(
                    "mcp/transport-error",
                    serde_json::json!({ "message": format!("SSE 连接失败：{e}") }),
                );
                return;
            }
        };
        if !response.status().is_success() {
            hub.push_notification(
                "mcp/transport-error",
                serde_json::json!({ "message": format!("SSE 连接失败：HTTP {}", response.status()) }),
            );
            return;
        }

        let mut events = sse_stream::SseStream::from_bytes_stream(response.bytes_stream());
        loop {
            use futures::StreamExt;
            let Some(event) = events.next().await else {
                break;
            };
            let event = match event {
                Ok(event) => event,
                Err(e) => {
                    hub.push_notification(
                        "mcp/transport-error",
                        serde_json::json!({ "message": format!("SSE 流读取失败：{e}") }),
                    );
                    break;
                }
            };
            match event.event.as_deref() {
                Some("endpoint") => {
                    let Some(data) = event.data.as_deref() else {
                        continue;
                    };
                    match base.join(data.trim()) {
                        Ok(endpoint) => {
                            let _ = endpoint_tx.send(Some(endpoint.clone()));
                            hub.push_notification(
                                "mcp/notice",
                                serde_json::json!({ "message": format!("SSE endpoint：{endpoint}") }),
                            );
                        }
                        Err(e) => {
                            hub.push_notification(
                                "mcp/transport-error",
                                serde_json::json!({ "message": format!("endpoint 地址解析失败：{e}") }),
                            );
                        }
                    }
                }
                Some("error") => {
                    hub.push_notification(
                        "mcp/transport-error",
                        serde_json::json!({ "message": event.data.clone().unwrap_or_default() }),
                    );
                }
                _ => {
                    // message 事件（JSON-RPC 服务端消息）
                    let Some(data) = event.data.as_deref() else {
                        continue;
                    };
                    match serde_json::from_str::<ServerJsonRpcMessage>(data) {
                        Ok(message) => {
                            if in_tx.try_send(message).is_err() {
                                hub.push_notification(
                                    "mcp/transport-error",
                                    serde_json::json!({ "message": "SSE 接收缓冲已满，消息被丢弃" }),
                                );
                            }
                        }
                        Err(e) => {
                            hub.push_notification(
                                "mcp/notice",
                                serde_json::json!({ "message": format!("无法解析 SSE 消息：{e}") }),
                            );
                        }
                    }
                }
            }
        }
        // 流结束：丢弃发送端 → 服务层感知传输关闭
        hub.push_notification(
            "mcp/transport-error",
            serde_json::json!({ "message": "SSE 事件流已关闭" }),
        );
    })
}

/// 发送消息：POST 到 endpoint（等待 endpoint 就绪）
fn spawn_sender(
    client: reqwest::Client,
    mut out_rx: mpsc::Receiver<TxJsonRpcMessage<RoleClient>>,
    mut endpoint_rx: watch::Receiver<Option<Url>>,
    hub: Arc<EventHub>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use futures::StreamExt;
        loop {
            let Some(message) = out_rx.next().await else {
                break;
            };
            // 等待 endpoint 事件到达（最多等 10s）
            let endpoint = match wait_endpoint(&mut endpoint_rx, Duration::from_secs(10)).await {
                Some(endpoint) => endpoint,
                None => {
                    hub.push_notification(
                        "mcp/transport-error",
                        serde_json::json!({ "message": "SSE endpoint 未就绪，消息发送失败" }),
                    );
                    break;
                }
            };
            let body = match serde_json::to_value(&message) {
                Ok(body) => body,
                Err(e) => {
                    hub.push_notification(
                        "mcp/transport-error",
                        serde_json::json!({ "message": format!("消息序列化失败：{e}") }),
                    );
                    continue;
                }
            };
            match client.post(endpoint).json(&body).send().await {
                Ok(response) => {
                    let status = response.status();
                    // 约定：POST 成功返回 202/200（响应消息经 SSE 流回传）
                    if !(status.is_success() || status.as_u16() == 202) {
                        hub.push_notification(
                            "mcp/transport-error",
                            serde_json::json!({ "message": format!("消息发送失败：HTTP {status}") }),
                        );
                    }
                }
                Err(e) => {
                    hub.push_notification(
                        "mcp/transport-error",
                        serde_json::json!({ "message": format!("消息发送失败：{e}") }),
                    );
                    break;
                }
            }
        }
    })
}

async fn wait_endpoint(
    endpoint_rx: &mut watch::Receiver<Option<Url>>,
    timeout: Duration,
) -> Option<Url> {
    if let Some(endpoint) = endpoint_rx.borrow().clone() {
        return Some(endpoint);
    }
    let wait = async {
        loop {
            if endpoint_rx.changed().await.is_err() {
                return None;
            }
            if let Some(endpoint) = endpoint_rx.borrow().clone() {
                return Some(endpoint);
            }
        }
    };
    tokio::time::timeout(timeout, wait).await.ok().flatten()
}
