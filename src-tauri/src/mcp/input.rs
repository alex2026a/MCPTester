//! 交互输入通道（v0.6.0）
//!
//! - 现代世代 MRTR 与旧世代 elicitation 共用 [`InputHandler`]；
//! - [`InputBroker`] 负责挂起、事件、超时、取消与待处理查询；
//! - [`ActiveCalls`] 把旧世代服务器请求尽力归属到正在进行的调用；
//! - 测试运行器/ probe 使用 [`RejectInputHandler`] 快速失败。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rmcp::model::{
    ElicitRequestParams, ElicitResult, ElicitationAction, InputRequest, InputRequests,
    InputResponses, NumberOrString, ProgressToken,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

use super::models::{CallMethod, ServerInput};

/// 用户等待输入的超时（秒）
pub const INPUT_WAIT_TIMEOUT_SECS: u64 = 300;
/// 单轮最多输入请求数
pub const MAX_INPUT_REQUESTS: usize = 8;
/// 单个 elicitation schema 上限
pub const MAX_INPUT_SCHEMA_BYTES: usize = 64 * 1024;
/// 同时挂起的输入请求上限（防止异常服务器拖垮内存）
const MAX_PENDING_REQUESTS: usize = 32;

/// 一次等待用户输入的上下文
#[derive(Serialize, Clone, Debug)]
pub struct InputContext {
    pub server_id: String,
    pub server_name: String,
    /// 旧世代可能无法归属到具体调用
    pub call_id: Option<String>,
    pub method: Option<CallMethod>,
    pub target: Option<String>,
    /// MRTR 轮次（1-based）；旧世代固定 1
    pub round: u32,
}

/// 只把 elicitation 投影给前端；sampling/roots 不展示
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputRequestView {
    Elicitation {
        key: String,
        /// "form" | "url"
        mode: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        elicitation_id: Option<String>,
    },
}

/// 输入事件（`mcp://input`）
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Opened {
        request_id: String,
        context: InputContext,
        requests: Vec<InputRequestView>,
        expires_at: DateTime<Utc>,
    },
    Closed {
        request_id: String,
        /// answered / timeout / canceled / disconnected / call_finished
        reason: String,
    },
}

/// 前端提交的单条应答
#[derive(Deserialize, Clone, Debug)]
pub struct InputAnswer {
    pub key: String,
    pub action: InputAction,
    #[serde(default)]
    pub content: Option<Value>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputAction {
    Accept,
    Decline,
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("不支持的交互输入：{kind}")]
    Unsupported { kind: String },
    #[error("等待交互输入超时（{timeout_secs} 秒）")]
    Timeout { timeout_secs: u64 },
    #[error("交互输入已取消")]
    Canceled,
    #[error("连接已断开")]
    Disconnected,
    #[error("输入请求不存在或已过期")]
    NoPending,
    #[error("输入应答不完整：{message}")]
    InvalidAnswer { message: String },
    #[error("交互输入不符合协议：{message}")]
    Invalid { message: String },
}

/// 输入处理器：UI、脚本测试、拒绝三种实现
#[async_trait]
pub trait InputHandler: Send + Sync {
    fn interactive(&self) -> bool {
        false
    }

    async fn request(
        &self,
        context: InputContext,
        requests: InputRequests,
        timeout: Duration,
    ) -> Result<InputResponses, InputError>;

    /// 用户取消某次调用
    fn cancel_call(&self, _call_id: &str) {}

    /// 调用结束（成功/超时/错误/取消）时清理待输入
    fn call_finished(&self, _call_id: &str) {}

    /// 连接关闭时清理该服务器的待输入
    fn connection_closed(&self, _server_id: &str) {}
}

/// UI 实现：写入 InputBroker 并等待前端应答
pub struct UiInputHandler {
    broker: Arc<InputBroker>,
}

impl UiInputHandler {
    pub fn new(broker: Arc<InputBroker>) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl InputHandler for UiInputHandler {
    fn interactive(&self) -> bool {
        true
    }

    async fn request(
        &self,
        context: InputContext,
        requests: InputRequests,
        timeout: Duration,
    ) -> Result<InputResponses, InputError> {
        self.broker.request(context, requests, timeout).await
    }

    fn cancel_call(&self, call_id: &str) {
        self.broker.cancel_call(call_id);
    }

    fn call_finished(&self, call_id: &str) {
        self.broker.finish_call(call_id);
    }

    fn connection_closed(&self, server_id: &str) {
        self.broker.connection_closed(server_id);
    }
}

/// 拒绝实现：测试运行器 / probe 使用
#[derive(Default, Clone, Copy)]
pub struct RejectInputHandler;

impl RejectInputHandler {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl InputHandler for RejectInputHandler {
    async fn request(
        &self,
        _context: InputContext,
        _requests: InputRequests,
        _timeout: Duration,
    ) -> Result<InputResponses, InputError> {
        Err(InputError::Unsupported {
            kind: "elicitation".to_string(),
        })
    }
}

/// 正在进行的调用（旧世代 elicitation 归属用）
pub struct ActiveCall {
    pub input: Arc<dyn InputHandler>,
    pub method: CallMethod,
    pub target: String,
    pub server_inputs: Mutex<Vec<ServerInput>>,
}

impl ActiveCall {
    pub fn new(
        input: Arc<dyn InputHandler>,
        method: CallMethod,
        target: impl Into<String>,
    ) -> Self {
        Self {
            input,
            method,
            target: target.into(),
            server_inputs: Mutex::new(Vec::new()),
        }
    }

    pub fn push_server_input(&self, input: ServerInput) {
        self.server_inputs
            .lock()
            .expect("服务器输入锁异常")
            .push(input);
    }

    pub fn take_server_inputs(&self) -> Vec<ServerInput> {
        std::mem::take(&mut *self.server_inputs.lock().expect("服务器输入锁异常"))
    }
}

/// 连接内活动调用表
#[derive(Default)]
pub struct ActiveCalls {
    map: Mutex<HashMap<String, Arc<ActiveCall>>>,
}

impl ActiveCalls {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(self: &Arc<Self>, call_id: String, call: Arc<ActiveCall>) -> ActiveCallGuard {
        self.map
            .lock()
            .expect("活动调用锁异常")
            .insert(call_id.clone(), call);
        ActiveCallGuard {
            calls: Arc::clone(self),
            call_id,
        }
    }

    pub fn get(&self, call_id: &str) -> Option<Arc<ActiveCall>> {
        self.map
            .lock()
            .expect("活动调用锁异常")
            .get(call_id)
            .cloned()
    }

    /// 只有一个活动调用时返回它；用于旧世代归属
    pub fn single(&self) -> Option<(String, Arc<ActiveCall>)> {
        let map = self.map.lock().expect("活动调用锁异常");
        if map.len() != 1 {
            return None;
        }
        map.iter()
            .next()
            .map(|(id, call)| (id.clone(), Arc::clone(call)))
    }

    /// 旧世代归属：唯一活动调用优先；progressToken 仅 best-effort
    pub fn resolve(
        &self,
        progress_token: Option<ProgressToken>,
    ) -> (Option<String>, Option<Arc<ActiveCall>>) {
        if let Some((id, call)) = self.single() {
            return (Some(id), Some(call));
        }
        if let Some(ProgressToken(NumberOrString::String(id))) = progress_token {
            let id = id.to_string();
            if let Some(call) = self.get(&id) {
                return (Some(id), Some(call));
            }
        }
        (None, None)
    }

    pub fn remove(&self, call_id: &str) {
        self.map
            .lock()
            .expect("活动调用锁异常")
            .remove(call_id);
    }

    pub fn clear(&self) {
        self.map.lock().expect("活动调用锁异常").clear();
    }
}

/// 活动调用守卫：所有早退路径都会移除
pub struct ActiveCallGuard {
    calls: Arc<ActiveCalls>,
    call_id: String,
}

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        self.calls.remove(&self.call_id);
    }
}

/// 单次调用的输入清理守卫：结束/取消时关闭待输入
pub struct CallInputGuard {
    input: Arc<dyn InputHandler>,
    call_id: String,
}

impl CallInputGuard {
    pub fn new(input: Arc<dyn InputHandler>, call_id: impl Into<String>) -> Self {
        Self {
            input,
            call_id: call_id.into(),
        }
    }
}

impl Drop for CallInputGuard {
    fn drop(&mut self) {
        self.input.call_finished(&self.call_id);
    }
}

struct PendingInput {
    event: InputEvent,
    requests: InputRequests,
    tx: oneshot::Sender<Result<InputResponses, InputError>>,
}

/// 应用级输入挂起表 + 事件广播
pub struct InputBroker {
    pending: Mutex<HashMap<String, PendingInput>>,
    events: broadcast::Sender<InputEvent>,
}

impl InputBroker {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            pending: Mutex::new(HashMap::new()),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<InputEvent> {
        self.events.subscribe()
    }

    fn emit(&self, event: InputEvent) {
        let _ = self.events.send(event);
    }

    pub async fn request(
        &self,
        context: InputContext,
        requests: InputRequests,
        timeout: Duration,
    ) -> Result<InputResponses, InputError> {
        let views = self.validate_requests(&requests)?;
        let request_id = Uuid::new_v4().to_string();
        let expires_at = Utc::now()
            + chrono::Duration::from_std(timeout)
                .unwrap_or_else(|_| chrono::Duration::seconds(INPUT_WAIT_TIMEOUT_SECS as i64));
        let event = InputEvent::Opened {
            request_id: request_id.clone(),
            context,
            requests: views,
            expires_at,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("输入挂起锁异常");
            if pending.len() >= MAX_PENDING_REQUESTS {
                return Err(InputError::Invalid {
                    message: format!("同时挂起的输入请求超过 {MAX_PENDING_REQUESTS} 条"),
                });
            }
            pending.insert(
                request_id.clone(),
                PendingInput {
                    event: event.clone(),
                    requests,
                    tx,
                },
            );
        }
        self.emit(event);
        let guard = PendingGuard {
            broker: self,
            request_id: request_id.clone(),
        };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(responses))) => Ok(responses),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(InputError::Canceled),
            Err(_) => {
                self.remove_pending(&request_id, "timeout");
                drop(guard);
                Err(InputError::Timeout {
                    timeout_secs: timeout.as_secs().max(1),
                })
            }
        }
    }

    /// 前端提交应答；校验失败时保持 pending
    pub fn respond(&self, request_id: &str, answers: Vec<InputAnswer>) -> Result<(), InputError> {
        let mut pending = self.pending.lock().expect("输入挂起锁异常");
        let Some(entry) = pending.get(request_id) else {
            return Err(InputError::NoPending);
        };
        let responses = build_responses(&entry.requests, &answers)?;
        let entry = pending.remove(request_id).expect("输入挂起条目消失");
        drop(pending);
        let _ = entry.tx.send(Ok(responses));
        self.emit(InputEvent::Closed {
            request_id: request_id.to_string(),
            reason: "answered".to_string(),
        });
        Ok(())
    }

    /// 取消单条输入；返回上下文供命令层继续取消整个调用
    pub fn cancel(&self, request_id: &str) -> Option<InputContext> {
        let mut pending = self.pending.lock().expect("输入挂起锁异常");
        let entry = pending.remove(request_id)?;
        let context = event_context(&entry.event);
        drop(pending);
        let _ = entry.tx.send(Err(InputError::Canceled));
        self.emit(InputEvent::Closed {
            request_id: request_id.to_string(),
            reason: "canceled".to_string(),
        });
        context
    }

    pub fn cancel_call(&self, call_id: &str) {
        let ids = self.ids_for(|context| context.call_id.as_deref() == Some(call_id));
        for id in ids {
            self.cancel(&id);
        }
    }

    pub fn finish_call(&self, call_id: &str) {
        let ids = self.ids_for(|context| context.call_id.as_deref() == Some(call_id));
        for id in ids {
            self.remove_pending(&id, "call_finished");
        }
    }

    pub fn connection_closed(&self, server_id: &str) {
        let ids = self.ids_for(|context| context.server_id == server_id);
        for id in ids {
            self.remove_pending_with(&id, "disconnected", InputError::Disconnected);
        }
    }

    pub fn list_pending(&self) -> Vec<InputEvent> {
        self.pending
            .lock()
            .expect("输入挂起锁异常")
            .values()
            .map(|entry| entry.event.clone())
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().expect("输入挂起锁异常").len()
    }

    fn ids_for(&self, predicate: impl Fn(&InputContext) -> bool) -> Vec<String> {
        let pending = self.pending.lock().expect("输入挂起锁异常");
        pending
            .iter()
            .filter_map(|(id, entry)| {
                event_context(&entry.event)
                    .filter(|context| predicate(context))
                    .map(|_| id.clone())
            })
            .collect()
    }

    fn remove_pending(&self, request_id: &str, reason: &str) {
        self.remove_pending_with(request_id, reason, InputError::Canceled);
    }

    fn remove_pending_with(&self, request_id: &str, reason: &str, error: InputError) {
        let entry = {
            let mut pending = self.pending.lock().expect("输入挂起锁异常");
            pending.remove(request_id)
        };
        let Some(entry) = entry else {
            return;
        };
        let _ = entry.tx.send(Err(error));
        self.emit(InputEvent::Closed {
            request_id: request_id.to_string(),
            reason: reason.to_string(),
        });
    }

    fn validate_requests(
        &self,
        requests: &InputRequests,
    ) -> Result<Vec<InputRequestView>, InputError> {
        if requests.is_empty() {
            return Err(InputError::Invalid {
                message: "input_required 没有输入请求".to_string(),
            });
        }
        if requests.len() > MAX_INPUT_REQUESTS {
            return Err(InputError::Invalid {
                message: format!(
                    "单轮输入请求 {} 条，超过上限 {MAX_INPUT_REQUESTS}",
                    requests.len()
                ),
            });
        }
        let mut views = Vec::with_capacity(requests.len());
        for (key, request) in requests {
            match request {
                InputRequest::Elicitation(request) => match &request.params {
                    ElicitRequestParams::FormElicitationParams {
                        message,
                        requested_schema,
                        ..
                    } => {
                        let schema = serde_json::to_value(requested_schema).map_err(|error| {
                            InputError::Invalid {
                                message: format!("elicitation schema 序列化失败：{error}"),
                            }
                        })?;
                        let bytes = serde_json::to_vec(&schema).map(|value| value.len()).unwrap_or(0);
                        if bytes > MAX_INPUT_SCHEMA_BYTES {
                            return Err(InputError::Invalid {
                                message: format!(
                                    "elicitation schema {bytes} 字节，超过上限 {MAX_INPUT_SCHEMA_BYTES}"
                                ),
                            });
                        }
                        views.push(InputRequestView::Elicitation {
                            key: key.clone(),
                            mode: "form".to_string(),
                            message: message.clone(),
                            schema: Some(schema),
                            url: None,
                            elicitation_id: None,
                        });
                    }
                    ElicitRequestParams::UrlElicitationParams {
                        message,
                        url,
                        elicitation_id,
                        ..
                    } => {
                        views.push(InputRequestView::Elicitation {
                            key: key.clone(),
                            mode: "url".to_string(),
                            message: message.clone(),
                            schema: None,
                            url: Some(url.clone()),
                            elicitation_id: Some(elicitation_id.clone()),
                        });
                    }
                    _ => {
                        return Err(InputError::Unsupported {
                            kind: "elicitation".to_string(),
                        });
                    }
                },
                InputRequest::CreateMessage(_) => {
                    return Err(InputError::Unsupported {
                        kind: "sampling".to_string(),
                    });
                }
                InputRequest::ListRoots(_) => {
                    return Err(InputError::Unsupported {
                        kind: "roots".to_string(),
                    });
                }
                _ => {
                    return Err(InputError::Unsupported {
                        kind: "unknown".to_string(),
                    });
                }
            }
        }
        Ok(views)
    }
}

impl Default for InputBroker {
    fn default() -> Self {
        Self::new()
    }
}

struct PendingGuard<'a> {
    broker: &'a InputBroker,
    request_id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.broker.remove_pending(&self.request_id, "canceled");
    }
}

fn event_context(event: &InputEvent) -> Option<InputContext> {
    match event {
        InputEvent::Opened { context, .. } => Some(context.clone()),
        InputEvent::Closed { .. } => None,
    }
}

fn build_responses(
    requests: &InputRequests,
    answers: &[InputAnswer],
) -> Result<InputResponses, InputError> {
    if answers.len() != requests.len() {
        return Err(InputError::InvalidAnswer {
            message: format!(
                "应答 {} 条，请求 {} 条",
                answers.len(),
                requests.len()
            ),
        });
    }
    let mut seen = HashSet::new();
    let mut responses = BTreeMap::new();
    for answer in answers {
        if !seen.insert(answer.key.clone()) {
            return Err(InputError::InvalidAnswer {
                message: format!("重复的输入 key：{}", answer.key),
            });
        }
        let Some(request) = requests.get(&answer.key) else {
            return Err(InputError::InvalidAnswer {
                message: format!("未知的输入 key：{}", answer.key),
            });
        };
        let InputRequest::Elicitation(request) = request else {
            return Err(InputError::Unsupported {
                kind: "non-elicitation".to_string(),
            });
        };
        let form = matches!(
            &request.params,
            ElicitRequestParams::FormElicitationParams { .. }
        );
        let action = match answer.action {
            InputAction::Accept => ElicitationAction::Accept,
            InputAction::Decline => ElicitationAction::Decline,
        };
        let mut result = ElicitResult::new(action);
        if answer.action == InputAction::Accept && form {
            let content = answer.content.clone().ok_or_else(|| InputError::InvalidAnswer {
                message: format!("{} 接受输入但缺少 content", answer.key),
            })?;
            if !content.is_object() {
                return Err(InputError::InvalidAnswer {
                    message: format!("{} 的 content 必须是 JSON 对象", answer.key),
                });
            }
            result = result.with_content(content);
        }
        let value = serde_json::to_value(result).map_err(|error| InputError::Invalid {
            message: format!("elicitation 结果序列化失败：{error}"),
        })?;
        responses.insert(answer.key.clone(), value);
    }
    Ok(responses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{ElicitRequest, ElicitationSchema};
    use serde_json::json;

    fn form_request() -> InputRequests {
        let schema: ElicitationSchema = serde_json::from_value(json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
        }))
        .expect("schema");
        let params = ElicitRequestParams::FormElicitationParams {
            meta: None,
            message: "请输入名称".to_string(),
            requested_schema: schema,
        };
        let mut requests = InputRequests::new();
        requests.insert(
            "name".to_string(),
            InputRequest::Elicitation(ElicitRequest::new(params)),
        );
        requests
    }

    fn context(call_id: Option<&str>) -> InputContext {
        InputContext {
            server_id: "server".to_string(),
            server_name: "server".to_string(),
            call_id: call_id.map(str::to_string),
            method: Some(CallMethod::ToolsCall),
            target: Some("echo".to_string()),
            round: 1,
        }
    }

    #[tokio::test]
    async fn broker_responds_and_closes() {
        let broker = Arc::new(InputBroker::new());
        let mut events = broker.subscribe();
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move {
                broker
                    .request(context(Some("call-1")), form_request(), Duration::from_secs(5))
                    .await
            })
        };
        let InputEvent::Opened { request_id, .. } = events.recv().await.expect("opened") else {
            panic!("expected opened");
        };
        broker
            .respond(
                &request_id,
                vec![InputAnswer {
                    key: "name".to_string(),
                    action: InputAction::Accept,
                    content: Some(json!({ "name": "Ada" })),
                }],
            )
            .expect("respond");
        let responses = task.await.expect("task").expect("responses");
        assert_eq!(responses["name"]["action"], "accept");
        assert!(matches!(
            events.recv().await.expect("closed"),
            InputEvent::Closed { reason, .. } if reason == "answered"
        ));
    }

    #[tokio::test]
    async fn broker_rejects_missing_key() {
        let broker = Arc::new(InputBroker::new());
        let mut events = broker.subscribe();
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move {
                broker
                    .request(context(None), form_request(), Duration::from_secs(5))
                    .await
            })
        };
        let InputEvent::Opened { request_id, .. } = events.recv().await.expect("opened") else {
            panic!("expected opened");
        };
        let error = broker
            .respond(&request_id, Vec::new())
            .expect_err("should reject");
        assert!(matches!(error, InputError::InvalidAnswer { .. }));
        assert_eq!(broker.pending_count(), 1);
        broker.cancel(&request_id);
        assert!(matches!(task.await, Ok(Err(InputError::Canceled))));
    }

    #[tokio::test]
    async fn broker_timeout_cleans_pending() {
        let broker = Arc::new(InputBroker::new());
        let result = broker
            .request(
                context(None),
                form_request(),
                Duration::from_millis(20),
            )
            .await;
        assert!(matches!(result, Err(InputError::Timeout { .. })));
        assert_eq!(broker.pending_count(), 0);
    }

    #[tokio::test]
    async fn broker_cancel_call_cleans_pending() {
        let broker = Arc::new(InputBroker::new());
        let mut events = broker.subscribe();
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move {
                broker
                    .request(context(Some("call-1")), form_request(), Duration::from_secs(5))
                    .await
            })
        };
        let _ = events.recv().await.expect("opened");
        broker.cancel_call("call-1");
        assert_eq!(broker.pending_count(), 0);
        assert!(matches!(task.await, Ok(Err(InputError::Canceled))));
    }
}
