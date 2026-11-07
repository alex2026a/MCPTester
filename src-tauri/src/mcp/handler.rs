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
    CancelledNotificationParam, ClientCapabilities, ClientConfig, ElicitRequest,
    ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationCapability, ErrorCode,
    ErrorData, FormElicitationCapability, Implementation, InputRequest, InputRequests,
    LoggingMessageNotificationParam, ProgressNotificationParam, ResourceUpdatedNotificationParam,
    SubscriptionsAcknowledgedNotificationParams, UrlElicitationCapability,
};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use serde::{Deserialize, Serialize};

use super::input::{
    ActiveCalls, InputContext, InputError, InputHandler, INPUT_WAIT_TIMEOUT_SECS,
};
use super::models::{RuntimeSettings, ServerInput};

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
    server_name: String,
    input: Arc<dyn InputHandler>,
    active: Arc<ActiveCalls>,
}

impl AppClientHandler {
    pub fn new(
        hub: Arc<EventHub>,
        server_name: impl Into<String>,
        input: Arc<dyn InputHandler>,
        active: Arc<ActiveCalls>,
    ) -> Self {
        Self {
            hub,
            server_name: server_name.into(),
            input,
            active,
        }
    }

    fn record<T: Serialize>(&self, method: &str, params: &T) {
        let value = serde_json::to_value(params).unwrap_or(serde_json::Value::Null);
        self.hub.push_notification(method, value);
    }
}

#[allow(deprecated)] // logging 通知在 2026-07-28 已弃用，但服务器仍可能发送，需展示
impl ClientHandler for AppClientHandler {
    fn get_info(&self) -> ClientConfig {
        // v0.6.0：交互连接声明 elicitation；只声明 elicitation，不声明 sampling/roots
        let mut capabilities = ClientCapabilities::default();
        if self.input.interactive() {
            capabilities.elicitation = Some(
                ElicitationCapability::new()
                    .with_form(FormElicitationCapability::new().with_schema_validation(true))
                    .with_url(UrlElicitationCapability::new()),
            );
        }
        ClientConfig::new(
            capabilities,
            Implementation::new("MCP Tester", env!("CARGO_PKG_VERSION")),
        )
    }

    fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<ElicitResult, ErrorData>> + Send + '_ {
        // 旧世代 elicitation：记录请求，挂起等待 UI，拒绝/取消/超时都有明确结果
        self.record("elicitation/create", &request);
        let hub = Arc::clone(&self.hub);
        let input = Arc::clone(&self.input);
        let active = Arc::clone(&self.active);
        let server_name = self.server_name.clone();
        let request_value = serde_json::to_value(&request).unwrap_or(serde_json::Value::Null);
        async move {
            let (call_id, active_call) = active.resolve(context.meta.get_progress_token());
            let context_view = InputContext {
                server_id: hub.server_id.clone(),
                server_name,
                call_id: call_id.clone(),
                method: active_call.as_ref().map(|call| call.method.clone()),
                target: active_call.as_ref().map(|call| call.target.clone()),
                round: 1,
            };
            let mut requests = InputRequests::new();
            requests.insert(
                "elicitation".to_string(),
                InputRequest::Elicitation(ElicitRequest::new(request.clone())),
            );
            let result = tokio::select! {
                result = input.request(
                    context_view,
                    requests,
                    std::time::Duration::from_secs(INPUT_WAIT_TIMEOUT_SECS),
                ) => result,
                _ = context.ct.cancelled() => Err(InputError::Canceled),
            };
            let response = match result {
                Ok(responses) => {
                    let value = responses
                        .get("elicitation")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::from_value::<ElicitResult>(value).map_err(|error| {
                        ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("elicitation 应答解析失败：{error}"),
                            None,
                        )
                    })?
                }
                Err(InputError::Unsupported { .. } | InputError::Invalid { .. }) => {
                    return Err(ErrorData::new(
                        ErrorCode::INVALID_REQUEST,
                        "当前运行模式不支持或无法处理交互输入",
                        None,
                    ));
                }
                Err(
                    InputError::Timeout { .. }
                    | InputError::Canceled
                    | InputError::Disconnected,
                ) => ElicitResult::new(ElicitationAction::Cancel),
                Err(InputError::NoPending | InputError::InvalidAnswer { .. }) => {
                    return Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        "交互输入应答不可用",
                        None,
                    ));
                }
            };
            if let Some(call) = active_call {
                call.push_server_input(ServerInput {
                    kind: "elicitation".to_string(),
                    request: request_value,
                    response: Some(
                        serde_json::to_value(&response).unwrap_or(serde_json::Value::Null),
                    ),
                    at: Utc::now(),
                    truncated: false,
                });
            }
            hub.push_notification(
                "elicitation/response",
                serde_json::to_value(&response).unwrap_or(serde_json::Value::Null),
            );
            Ok(response)
        }
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
