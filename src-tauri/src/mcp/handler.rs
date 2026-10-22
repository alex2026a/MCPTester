//! 通知扇入与事件汇聚（见设计文档 03 第 1/2 节）
//!
//! - 旧世代：通知经 `ClientHandler` 回调到达；
//! - 现代世代：`subscriptions/listen` 的 `Subscription` 收到通知（不经过 ClientHandler）；
//! - 两条来源统一扇入 [`EventHub`]（广播通道 + 环形缓冲），供 UI/测试消费。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rmcp::ClientHandler;
#[allow(deprecated)] // logging 相关类型在 2026-07-28 已弃用，但服务器仍可能发送
use rmcp::model::{
    CancelledNotificationParam, ClientCapabilities, ClientConfig, ElicitRequestParams, ElicitResult,
    ElicitationAction, ErrorCode, ErrorData, Implementation, LoggingMessageNotificationParam,
    ProgressNotificationParam, ResourceUpdatedNotificationParam,
    SubscriptionsAcknowledgedNotificationParams,
};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use serde::{Deserialize, Serialize};

use super::models::RuntimeSettings;

/// 通知缓冲条目（seq 连接内单调递增；连接重建后从 0 重新计数）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NotificationEntry {
    pub seq: u64,
    pub method: String,
    pub params: serde_json::Value,
    pub at: DateTime<Utc>,
}

/// stderr 行条目（仅内存缓冲，不落盘）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StderrLine {
    pub seq: u64,
    pub line: String,
    pub at: DateTime<Utc>,
}

/// 事件（供 Tauri 事件推送与测试断言）
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubEvent {
    Notification(NotificationEntry),
    Stderr(StderrLine),
}

/// 单个连接的事件汇聚：广播通道 + 环形缓冲
pub struct EventHub {
    pub server_id: String,
    tx: tokio::sync::broadcast::Sender<HubEvent>,
    notification_seq: AtomicU64,
    stderr_seq: AtomicU64,
    notifications: Mutex<VecDeque<NotificationEntry>>,
    stderr: Mutex<VecDeque<StderrLine>>,
    notification_capacity: usize,
    stderr_capacity: usize,
}

impl EventHub {
    pub fn new(server_id: impl Into<String>, settings: &RuntimeSettings) -> Arc<Self> {
        let (tx, _rx) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            server_id: server_id.into(),
            tx,
            notification_seq: AtomicU64::new(0),
            stderr_seq: AtomicU64::new(0),
            notifications: Mutex::new(VecDeque::new()),
            stderr: Mutex::new(VecDeque::new()),
            notification_capacity: settings.notification_buffer_lines.max(1),
            stderr_capacity: settings.stderr_buffer_lines.max(1),
        })
    }

    /// 订阅事件流（当前无订阅者时发送会失败，忽略即可）
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    fn emit(&self, event: HubEvent) {
        let _ = self.tx.send(event);
    }

    /// 记录一条服务端通知
    pub fn push_notification(&self, method: &str, params: serde_json::Value) {
        let entry = NotificationEntry {
            seq: self.notification_seq.fetch_add(1, Ordering::Relaxed) + 1,
            method: method.to_string(),
            params,
            at: Utc::now(),
        };
        {
            let mut buf = self.notifications.lock().expect("通知缓冲锁异常");
            buf.push_back(entry.clone());
            while buf.len() > self.notification_capacity {
                buf.pop_front();
            }
        }
        self.emit(HubEvent::Notification(entry));
    }

    /// 记录一行 stderr（仅内存）
    pub fn push_stderr(&self, line: impl Into<String>) {
        let entry = StderrLine {
            seq: self.stderr_seq.fetch_add(1, Ordering::Relaxed) + 1,
            line: line.into(),
            at: Utc::now(),
        };
        {
            let mut buf = self.stderr.lock().expect("stderr 缓冲锁异常");
            buf.push_back(entry.clone());
            while buf.len() > self.stderr_capacity {
                buf.pop_front();
            }
        }
        self.emit(HubEvent::Stderr(entry));
    }

    /// 当前通知最大 seq（无通知时为 0）
    pub fn last_notification_seq(&self) -> u64 {
        self.notification_seq.load(Ordering::Relaxed)
    }

    /// 当前 stderr 最大 seq（无输出时为 0）
    pub fn last_stderr_seq(&self) -> u64 {
        self.stderr_seq.load(Ordering::Relaxed)
    }

    /// 自 `since_seq` 之后的通知（0 = 全量）
    pub fn notification_buffer(&self, since_seq: u64) -> Vec<NotificationEntry> {
        let buf = self.notifications.lock().expect("通知缓冲锁异常");
        buf.iter().filter(|e| e.seq > since_seq).cloned().collect()
    }

    /// 自 `since_seq` 之后的 stderr 行（0 = 全量）
    pub fn stderr_buffer(&self, since_seq: u64) -> Vec<StderrLine> {
        let buf = self.stderr.lock().expect("stderr 缓冲锁异常");
        buf.iter().filter(|e| e.seq > since_seq).cloned().collect()
    }
}

/// 应用侧 ClientHandler：声明客户端信息，并把通知扇入 [`EventHub`]
#[derive(Clone)]
pub struct AppClientHandler {
    hub: Arc<EventHub>,
}

impl AppClientHandler {
    pub fn new(hub: Arc<EventHub>) -> Self {
        Self { hub }
    }

    fn record<T: Serialize>(&self, method: &str, params: &T) {
        let value = serde_json::to_value(params).unwrap_or(serde_json::Value::Null);
        self.hub.push_notification(method, value);
    }
}

#[allow(deprecated)] // logging 通知在 2026-07-28 已弃用，但服务器仍可能发送，需展示
impl ClientHandler for AppClientHandler {
    fn get_info(&self) -> ClientConfig {
        // v0.1 不声明客户端能力（roots / sampling / elicitation 均不支持）
        ClientConfig::new(ClientCapabilities::default(), Implementation::new("MCP Tester", env!("CARGO_PKG_VERSION")))
    }

    fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<ElicitResult, ErrorData>> + Send + '_ {
        // v0.1 不支持交互式输入：记录事件并婉拒（旧世代服务器）。
        // 现代世代该场景以 MRTR input_required 呈现，见调用管线。
        self.record("elicitation/create", &request);
        std::future::ready(Ok(ElicitResult::new(ElicitationAction::Decline)))
    }

    fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/cancelled", &params);
        std::future::ready(())
    }

    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/progress", &params);
        std::future::ready(())
    }

    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/message", &params);
        std::future::ready(())
    }

    fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/resources/updated", &params);
        std::future::ready(())
    }

    fn on_resource_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/resources/list_changed", &serde_json::Value::Null);
        std::future::ready(())
    }

    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/tools/list_changed", &serde_json::Value::Null);
        std::future::ready(())
    }

    fn on_prompt_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/prompts/list_changed", &serde_json::Value::Null);
        std::future::ready(())
    }

    fn on_subscriptions_acknowledged(
        &self,
        params: SubscriptionsAcknowledgedNotificationParams,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.record("notifications/subscriptions/acknowledged", &params);
        std::future::ready(())
    }
}

/// 未实现的客户端能力：统一返回 method not found（内部辅助）
#[allow(dead_code)]
pub fn unsupported_method(method: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::METHOD_NOT_FOUND,
        format!("v0.1 不支持客户端能力：{method}"),
        None,
    )
}
