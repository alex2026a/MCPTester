//! JSON-RPC 线上报文日志
//!
//! 实现方式：在连接层用 [`WireTransport`] 包装 rmcp [`Transport`]，
//! 双向 JSON-RPC 帧（initialize / discover / tools/call / 通知等）按序记录，
//! 三种传输（stdio / Streamable HTTP / SSE）统一覆盖。
//!
//! v0.1.3 起为"报文面板 = 会话时间线"补充派生字段：
//! - `kind`（request / response / notification / system）与 `method`、`rpc_id`；
//! - 请求 → 响应按 JSON-RPC id 配对，响应帧带 `duration_ms`；
//! - `bytes` / `is_error` 便于列表展示与错误标记；
//! - [`WireLog::since`] 支持增量读取（事件推送替代轮询）。
//!
//! 安全：仅内存存储；对报文做粗粒度脱敏（敏感键名 + Bearer/sk-/GitHub 等值形态），
//! 单条报文超过上限截断保存。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use serde::Serialize;
use serde_json::Value;

/// 报文方向
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireDirection {
    /// 客户端 → 服务器
    ClientToServer,
    /// 服务器 → 客户端
    ServerToClient,
    /// 本地系统事件（连接建立/断开等，便于对时）
    System,
}

/// 报文类型（按 JSON-RPC 帧形态分类）
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireKind {
    /// 带 id 的方法调用
    Request,
    /// 方法调用的应答（带 id）
    Response,
    /// 无 id 的单向通知（含服务端通知、无 id 的进度等）
    Notification,
    /// 本地系统事件（连接建立/断开等）
    System,
}

/// 报文条目
#[derive(Serialize, Clone, Debug)]
pub struct WireEntry {
    pub seq: u64,
    pub direction: WireDirection,
    /// 帧形态分类
    pub kind: WireKind,
    /// 方法名（request / notification 帧）
    pub method: Option<String>,
    /// JSON-RPC id（request / response 帧，统一转为字符串）
    pub rpc_id: Option<String>,
    /// 摘要（如 `request tools/call #3`、`response #3`）
    pub summary: String,
    /// 报文载荷（已脱敏；超限时截断为字符串）
    pub payload: Value,
    /// 是否因超限截断
    pub truncated: bool,
    /// 原始帧字节数（脱敏/截断前）
    pub bytes: usize,
    /// 应答是否为 error
    pub is_error: bool,
    /// 应答耗时（与请求配对成功时）
    pub duration_ms: Option<u64>,
    pub at: DateTime<Utc>,
}

/// 单连接报文日志（环形缓冲）
pub struct WireLog {
    seq: AtomicU64,
    capacity: usize,
    max_payload_bytes: usize,
    entries: Mutex<VecDeque<WireEntry>>,
    /// 在途请求（方向, id）→ 发送时刻，用于响应配对计时
    pending: Mutex<HashMap<(u8, String), DateTime<Utc>>>,
}

impl WireLog {
    pub fn new(capacity: usize, max_payload_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            seq: AtomicU64::new(0),
            capacity: capacity.max(10),
            max_payload_bytes: max_payload_bytes.max(1024),
            entries: Mutex::new(VecDeque::new()),
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// 记录一条报文（payload 会被脱敏并限长）
    pub fn push(&self, direction: WireDirection, summary: impl Into<String>, mut payload: Value) {
        let mut meta = frame_meta(&payload);
        if matches!(direction, WireDirection::System) {
            meta.kind = WireKind::System;
        }
        let bytes = serde_json::to_string(&payload)
            .map(|text| text.len())
            .unwrap_or(0);
        let at = Utc::now();
        let duration_ms = self.track(&direction, &meta, at);

        sanitize(&mut payload);
        let (payload, truncated) = bounded(payload, self.max_payload_bytes);

        let entry = WireEntry {
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            direction,
            kind: meta.kind,
            method: meta.method,
            rpc_id: meta.rpc_id,
            summary: summary.into(),
            payload,
            truncated,
            bytes,
            is_error: meta.is_error,
            duration_ms,
            at,
        };
        let mut buf = self.entries.lock().expect("报文缓冲锁异常");
        buf.push_back(entry);
        while buf.len() > self.capacity {
            buf.pop_front();
        }
    }

    /// 记录一条系统事件（连接建立/断开等）
    pub fn push_system(&self, message: impl Into<String>) {
        self.push(
            WireDirection::System,
            message,
            Value::Object(Default::default()),
        );
    }

    /// 请求/响应配对：请求记入在途表，响应到达时计算耗时
    fn track(&self, direction: &WireDirection, meta: &FrameMeta, at: DateTime<Utc>) -> Option<u64> {
        let id = meta.rpc_id.as_ref()?;
        let from_client = matches!(direction, WireDirection::ClientToServer);
        let mut pending = self.pending.lock().expect("报文配对表锁异常");
        match meta.kind {
            WireKind::Request => {
                pending.insert((from_client as u8, id.clone()), at);
                None
            }
            WireKind::Response => {
                let started = pending.remove(&((!from_client) as u8, id.clone()))?;
                let elapsed = (at - started).num_milliseconds();
                Some(elapsed.max(0) as u64)
            }
            _ => None,
        }
    }

    /// 全量快照（按 seq 升序）
    pub fn buffer(&self) -> Vec<WireEntry> {
        let buf = self.entries.lock().expect("报文缓冲锁异常");
        buf.iter().cloned().collect()
    }

    /// 增量快照：seq 大于 `after` 的报文（按 seq 升序）
    pub fn since(&self, after: u64) -> Vec<WireEntry> {
        let buf = self.entries.lock().expect("报文缓冲锁异常");
        buf.iter()
            .filter(|entry| entry.seq > after)
            .cloned()
            .collect()
    }

    /// 当前最大 seq（无报文时为 0）
    pub fn latest_seq(&self) -> u64 {
        let buf = self.entries.lock().expect("报文缓冲锁异常");
        buf.back().map(|entry| entry.seq).unwrap_or(0)
    }

    /// 清空
    pub fn clear(&self) {
        self.entries.lock().expect("报文缓冲锁异常").clear();
        self.pending.lock().expect("报文配对表锁异常").clear();
    }

    /// 当前条数
    pub fn len(&self) -> usize {
        self.entries.lock().expect("报文缓冲锁异常").len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 帧分类派生结果
struct FrameMeta {
    kind: WireKind,
    method: Option<String>,
    rpc_id: Option<String>,
    is_error: bool,
}

/// JSON-RPC id → 字符串（数字/字符串统一）
fn rpc_id_of(value: &Value) -> Option<String> {
    match value.get("id") {
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

/// 从 JSON-RPC 报文派生分类字段
fn frame_meta(value: &Value) -> FrameMeta {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rpc_id = rpc_id_of(value);
    let is_error = value.get("error").is_some();
    let kind = match (&method, &rpc_id) {
        (Some(_), Some(_)) => WireKind::Request,
        (Some(_), None) => WireKind::Notification,
        (None, Some(_)) => WireKind::Response,
        (None, None) => WireKind::Notification,
    };
    FrameMeta {
        kind,
        method,
        rpc_id,
        is_error,
    }
}

/// 从 JSON-RPC 报文派生摘要
pub fn summarize_message(value: &Value) -> String {
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        return match value.get("id") {
            None | Some(Value::Null) => format!("notification {method}"),
            Some(id) => format!("request {method} #{id}"),
        };
    }
    if let Some(id) = value.get("id") {
        if value.get("error").is_some() {
            return format!("error-response #{id}");
        }
        return format!("response #{id}");
    }
    "message".to_string()
}

/// 报文包装传输：所有帧经此转发并记录（覆盖三种传输）
pub struct WireTransport<T> {
    inner: T,
    wire: Arc<WireLog>,
}

impl<T> WireTransport<T> {
    pub fn new(inner: T, wire: Arc<WireLog>) -> Self {
        Self { inner, wire }
    }
}

impl<T> Transport<RoleClient> for WireTransport<T>
where
    T: Transport<RoleClient>,
{
    type Error = T::Error;

    fn name() -> std::borrow::Cow<'static, str> {
        T::name()
    }

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let value = serde_json::to_value(&item).unwrap_or(Value::Null);
        self.wire.push(
            WireDirection::ClientToServer,
            summarize_message(&value),
            value,
        );
        self.inner.send(item)
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        let wire = Arc::clone(&self.wire);
        async move {
            let item = self.inner.receive().await;
            if let Some(message) = &item {
                let value = serde_json::to_value(message).unwrap_or(Value::Null);
                wire.push(WireDirection::ServerToClient, summarize_message(&value), value);
            }
            item
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

// ---------------------------------------------------------------------------
// 脱敏与限长（纯函数，便于单测）
// ---------------------------------------------------------------------------

/// 敏感键名判定（粗粒度：包含即脱敏）
fn is_sensitive_key(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "authorization",
        "token",
        "secret",
        "password",
        "apikey",
        "api-key",
        "api_key",
        "credential",
    ];
    NEEDLES.iter().any(|needle| lowered.contains(needle))
}

/// 值形态兜底（Bearer / sk- / GitHub / Slack 令牌）
fn looks_like_secret(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("Bearer ")
        || trimmed.starts_with("sk-")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("github_pat_")
        || trimmed.starts_with("xoxb-")
        || trimmed.starts_with("xoxp-")
}

/// 递归脱敏（就地修改）
fn sanitize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *item = Value::String("****".to_string());
                } else {
                    sanitize(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                sanitize(item);
            }
        }
        Value::String(text) => {
            if looks_like_secret(text) {
                *text = "****".to_string();
            }
        }
        _ => {}
    }
}

/// 限长：超过 `max_bytes` 时截断为字符串预览并标记
fn bounded(value: Value, max_bytes: usize) -> (Value, bool) {
    let text = serde_json::to_string(&value).unwrap_or_default();
    if text.len() <= max_bytes {
        return (value, false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        Value::String(format!(
            "{}…（报文已截断，完整长度 {} 字节）",
            &text[..end],
            text.len()
        )),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_from_json_rpc() {
        assert_eq!(
            summarize_message(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call" })),
            "request tools/call #1"
        );
        assert_eq!(
            summarize_message(&json!({ "jsonrpc": "2.0", "method": "notifications/progress" })),
            "notification notifications/progress"
        );
        assert_eq!(
            summarize_message(&json!({ "jsonrpc": "2.0", "id": 2, "result": {} })),
            "response #2"
        );
        assert_eq!(
            summarize_message(&json!({ "jsonrpc": "2.0", "id": 3, "error": { "code": -1 } })),
            "error-response #3"
        );
    }

    #[test]
    fn frame_meta_classifies_frames() {
        let request = frame_meta(&json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call" }));
        assert_eq!(request.kind, WireKind::Request);
        assert_eq!(request.method.as_deref(), Some("tools/call"));
        assert_eq!(request.rpc_id.as_deref(), Some("7"));
        assert!(!request.is_error);

        let notification = frame_meta(&json!({ "method": "notifications/message" }));
        assert_eq!(notification.kind, WireKind::Notification);
        assert!(notification.rpc_id.is_none());

        let response = frame_meta(&json!({ "id": "abc", "result": {} }));
        assert_eq!(response.kind, WireKind::Response);
        assert_eq!(response.rpc_id.as_deref(), Some("abc"));

        let error = frame_meta(&json!({ "id": 1, "error": { "code": -32601 } }));
        assert!(error.is_error);
    }

    #[test]
    fn pairing_computes_duration_and_bytes() {
        let log = WireLog::new(10, 4096);
        log.push(
            WireDirection::ClientToServer,
            "request tools/call #1",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {} }),
        );
        log.push(
            WireDirection::ServerToClient,
            "response #1",
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }),
        );
        let buf = log.buffer();
        assert_eq!(buf[0].kind, WireKind::Request);
        assert!(buf[0].duration_ms.is_none());
        assert!(buf[0].bytes > 0);
        assert_eq!(buf[1].kind, WireKind::Response);
        assert!(buf[1].duration_ms.is_some(), "响应应携带配对耗时");
        assert!(!buf[1].is_error);
    }

    #[test]
    fn pairing_ignores_cross_direction_id_collision() {
        let log = WireLog::new(10, 4096);
        // 服务端请求 id=1（服务器 → 客户端）
        log.push(
            WireDirection::ServerToClient,
            "request sampling/createMessage #1",
            json!({ "id": 1, "method": "sampling/createMessage" }),
        );
        // 客户端对服务端请求的应答（客户端 → 服务器）
        log.push(
            WireDirection::ClientToServer,
            "response #1",
            json!({ "id": 1, "result": { "role": "assistant" } }),
        );
        let buf = log.buffer();
        assert_eq!(buf[1].kind, WireKind::Response);
        assert!(buf[1].duration_ms.is_some());
    }

    #[test]
    fn system_entries_are_marked() {
        let log = WireLog::new(10, 1024);
        log.push_system("连接已建立");
        let buf = log.buffer();
        assert_eq!(buf[0].kind, WireKind::System);
        assert_eq!(buf[0].direction, WireDirection::System);
    }

    #[test]
    fn since_returns_only_new_entries() {
        let log = WireLog::new(10, 1024);
        for i in 0..5 {
            log.push(WireDirection::ClientToServer, format!("request #{i}"), json!({ "i": i }));
        }
        assert_eq!(log.latest_seq(), 5);
        let tail = log.since(2);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].seq, 3);
        assert!(log.since(5).is_empty());
    }

    #[test]
    fn sanitize_redacts_keys_and_patterns() {
        let mut value = json!({
            "headers": { "Authorization": "Bearer abcdef", "X-Trace": "t1" },
            "token": "plain",
            "nested": { "value": "sk-abcdefgh" },
            "list": ["Bearer xyz", "safe"]
        });
        sanitize(&mut value);
        assert_eq!(value["headers"]["Authorization"], "****");
        assert_eq!(value["headers"]["X-Trace"], "t1");
        assert_eq!(value["token"], "****");
        assert_eq!(value["nested"]["value"], "****");
        assert_eq!(value["list"][0], "****");
        assert_eq!(value["list"][1], "safe");
    }

    #[test]
    fn bounded_truncates_oversize() {
        let big = json!({ "data": "x".repeat(4096) });
        let (value, truncated) = bounded(big, 1024);
        assert!(truncated);
        assert!(value.as_str().expect("应为字符串预览").contains("已截断"));
    }

    #[test]
    fn ring_buffer_capacity_and_order() {
        let log = WireLog::new(10, 1024);
        for i in 0..25 {
            log.push(WireDirection::ClientToServer, format!("request #{i}"), json!({ "i": i }));
        }
        let buf = log.buffer();
        assert_eq!(buf.len(), 10);
        assert_eq!(buf[0].payload["i"], 15);
        assert_eq!(buf[9].payload["i"], 24);
        assert!(buf.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn clear_resets_buffer_and_pending() {
        let log = WireLog::new(10, 1024);
        log.push(
            WireDirection::ClientToServer,
            "request #1",
            json!({ "id": 1, "method": "ping" }),
        );
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.latest_seq(), 0);
        // 清空后不应再与旧请求配对
        log.push(
            WireDirection::ServerToClient,
            "response #1",
            json!({ "id": 1, "result": {} }),
        );
        assert!(log.buffer()[0].duration_ms.is_none());
    }
}
