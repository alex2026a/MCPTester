//! 单服务器连接：生命周期、能力枚举、调用管线（见设计文档 03 第 3/5 节）

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use rmcp::ClientServiceExt;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, GetPromptRequest, GetPromptRequestParams,
    PingRequest, ProgressToken, ReadResourceRequest, ReadResourceRequestParams, RequestMetaObject,
    ServerPeerInfo, ServerResult, SubscriptionFilter,
};
use rmcp::service::{
    ClientLifecycleMode, PeerRequestOptions, RoleClient, RunningService, ServiceError,
};
use rmcp::transport::IntoTransport;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

use super::error::{CallError, McpError};
use super::handler::{AppClientHandler, EventHub};
use super::models::{
    CallMethod, CallRecord, CallStatus, CapabilitySnapshot, ConnectionState, Handshake, HttpConfig,
    LEGACY_FALLBACK_VERSION, LifecycleEra, PREFERRED_PROTOCOL_VERSIONS, PromptArgument, PromptInfo,
    ProbeResult, ResourceInfo, ResourceTemplateInfo, RuntimeSettings, StdioConfig, ToolInfo,
    TransportKind,
};
use super::transport;
use super::wire::{WireLog, WireTransport};

/// 在途调用条目（取消 = 中止本地等待 + 发送取消通知）
struct InFlight {
    abort: tokio::task::AbortHandle,
}

/// 连接句柄（每个服务器一个；内部状态并发安全）
pub struct ConnectionHandle {
    pub server_id: String,
    pub server_name: String,
    pub protocol_version_pin: Option<String>,
    transport_kind: RwLock<TransportKind>,
    hub: Arc<EventHub>,
    /// 线上报文日志（v0.1.1；所有传输统一在 Transport 层采集）
    wire: Arc<WireLog>,
    settings: RuntimeSettings,
    state: RwLock<ConnectionState>,
    service: Mutex<Option<RunningService<RoleClient, AppClientHandler>>>,
    capabilities: RwLock<Option<CapabilitySnapshot>>,
    inflight: Mutex<std::collections::HashMap<String, InFlight>>,
    subscription_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    monitor_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// SSE 传输工作线程（断开时中止）
    sse_workers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ConnectionHandle {
    pub fn new(
        server_id: impl Into<String>,
        server_name: impl Into<String>,
        protocol_version_pin: Option<String>,
        transport_kind: TransportKind,
        settings: RuntimeSettings,
    ) -> Arc<Self> {
        let server_id = server_id.into();
        let hub = EventHub::new(server_id.clone(), &settings);
        let wire = WireLog::new(settings.wire_buffer_lines, 16 * 1024);
        Arc::new(Self {
            server_id,
            server_name: server_name.into(),
            protocol_version_pin,
            transport_kind: RwLock::new(transport_kind),
            hub,
            wire,
            settings,
            state: RwLock::new(ConnectionState::Disconnected),
            service: Mutex::new(None),
            capabilities: RwLock::new(None),
            inflight: Mutex::new(std::collections::HashMap::new()),
            subscription_task: Mutex::new(None),
            monitor_task: Mutex::new(None),
            sse_workers: Mutex::new(Vec::new()),
        })
    }

    /// 事件汇聚（stderr / 通知缓冲；UI 事件推送在 M4 接入）
    pub fn hub(&self) -> &Arc<EventHub> {
        &self.hub
    }

    /// 线上报文日志（v0.1.1）
    pub fn wire(&self) -> &Arc<WireLog> {
        &self.wire
    }

    /// 当前连接状态
    pub async fn state(&self) -> ConnectionState {
        self.state.read().await.clone()
    }

    /// 当前传输类型
    pub async fn transport_kind(&self) -> TransportKind {
        self.transport_kind.read().await.clone()
    }

    async fn set_state(&self, state: ConnectionState) {
        *self.state.write().await = state;
    }

    /// 协商模式：默认 Auto（双世代探测）；`protocol_version_pin` 支持固定现代版本或强制旧世代握手
    fn lifecycle_mode(&self) -> ClientLifecycleMode {
        match self.protocol_version_pin.as_deref() {
            None => ClientLifecycleMode::Auto {
                preferred_versions: PREFERRED_PROTOCOL_VERSIONS.to_vec(),
                legacy_version: Some(LEGACY_FALLBACK_VERSION),
            },
            Some("2026-07-28") => ClientLifecycleMode::Discover {
                preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
            },
            // 其余版本按"强制旧世代 initialize 握手"处理（服务器回显其支持版本）
            Some(_) => ClientLifecycleMode::Initialize,
        }
    }

    /// 连接 stdio 服务器
    pub async fn connect_stdio(self: &Arc<Self>, cfg: &StdioConfig) -> Result<(), McpError> {
        *self.transport_kind.write().await = TransportKind::Stdio;
        let transport = transport::stdio::build_stdio(cfg, &self.hub)?;
        self.start(transport).await
    }

    /// 连接 Streamable HTTP 服务器
    pub async fn connect_http(self: &Arc<Self>, cfg: &HttpConfig) -> Result<(), McpError> {
        *self.transport_kind.write().await = TransportKind::StreamableHttp;
        let transport = transport::http::build_http(cfg, self.settings.request_timeout_secs)?;
        self.start(transport).await
    }

    /// 连接遗留 HTTP+SSE 服务器（规范已弃用，兼容旧服务器）
    pub async fn connect_sse(self: &Arc<Self>, cfg: &HttpConfig) -> Result<(), McpError> {
        *self.transport_kind.write().await = TransportKind::Sse;
        let transport =
            transport::sse::build_sse(cfg, self.settings.request_timeout_secs, &self.hub)?;
        *self.sse_workers.lock().await = transport.workers;
        self.start((transport.sink, transport.stream)).await
    }

    /// 建立生命周期（连接 + 协商）并启动监控/订阅
    async fn start<T, E, A>(self: &Arc<Self>, transport: T) -> Result<(), McpError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.set_state(ConnectionState::Connecting).await;

        let handler = AppClientHandler::new(Arc::clone(&self.hub));
        let mode = self.lifecycle_mode();
        // v0.1.1：统一在 Transport 层采集线上报文（三传输全覆盖）
        let inner = transport.into_transport();
        let transport = WireTransport::new(inner, Arc::clone(&self.wire));
        let init = handler.serve_with_lifecycle(transport, mode);
        let connect_timeout = Duration::from_secs(self.settings.connect_timeout_secs.max(1));

        let service = match tokio::time::timeout(connect_timeout, init).await {
            Err(_) => {
                let err = McpError::ConnectTimeout {
                    timeout_secs: self.settings.connect_timeout_secs,
                };
                self.wire
                    .push_system(format!("连接失败：{}", err.message()));
                self.set_state(ConnectionState::Error {
                    error: err.clone(),
                })
                .await;
                return Err(err);
            }
            Ok(Err(e)) => {
                let err = map_initialize_error(&e);
                self.wire
                    .push_system(format!("连接失败：{}", err.message()));
                self.set_state(ConnectionState::Error {
                    error: err.clone(),
                })
                .await;
                return Err(err);
            }
            Ok(Ok(service)) => service,
        };

        let handshake = build_handshake(service.peer().peer_info().as_deref());
        let era = handshake.lifecycle.clone();

        *self.service.lock().await = Some(service);
        self.set_state(ConnectionState::Ready {
            handshake: handshake.clone(),
        })
        .await;

        self.wire.push_system(format!(
            "连接建立：{} v{} · 协议 {} · {}",
            handshake.server_name,
            handshake.server_version,
            handshake.protocol_version,
            match era {
                LifecycleEra::Modern => "现代世代",
                LifecycleEra::Legacy => "旧世代",
            }
        ));

        tracing::info!(
            server_id = %self.server_id,
            protocol = %handshake.protocol_version,
            lifecycle = ?era,
            "MCP 连接建立"
        );

        // 现代世代：订阅列表变更（旧世代走上层 ClientHandler 回调）
        if era == LifecycleEra::Modern
            && let Err(e) = self.subscribe_list_changed().await
        {
            // 订阅失败不影响连接可用性（降级为手动刷新）
            tracing::warn!(server_id = %self.server_id, error = %e, "列表变更订阅失败");
            self.hub
                .push_notification("mcp/notice", serde_json::json!({ "message": format!("列表变更订阅失败：{e}") }));
        }

        self.start_monitor().await;
        Ok(())
    }

    /// 断开连接：取消在途调用 → 中止订阅 → 优雅关停（超时由 rmcp 处理）
    pub async fn disconnect(&self) {
        // 1. 取消在途调用（中止本地等待；协议层取消通知尽力而为）
        let entries: Vec<InFlight> = self.inflight.lock().await.drain().map(|(_, e)| e).collect();
        for entry in entries {
            entry.abort.abort();
        }

        // 2. 中止订阅任务
        if let Some(task) = self.subscription_task.lock().await.take() {
            task.abort();
        }
        // 2b. 中止 SSE 传输工作线程
        for task in self.sse_workers.lock().await.drain(..) {
            task.abort();
        }
        // 3. 中止监控任务
        if let Some(task) = self.monitor_task.lock().await.take() {
            task.abort();
        }

        // 4. 优雅关停服务（cancel 会触发 transport close / 子进程 graceful shutdown）
        let service = self.service.lock().await.take();
        if let Some(service) = service {
            let _ = tokio::time::timeout(Duration::from_secs(5), service.cancel()).await;
        }

        *self.capabilities.write().await = None;
        self.set_state(ConnectionState::Disconnected).await;
        self.wire.push_system("连接已断开");
    }

    /// 服务器进程退出 / 传输关闭监控
    async fn start_monitor(self: &Arc<Self>) {
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let closed = {
                    let guard = this.service.lock().await;
                    match guard.as_ref() {
                        // is_closed：显式关闭/取消；is_transport_closed：服务循环因传输结束而退出（子进程退出等）
                        Some(service) => {
                            service.is_closed() || service.peer().is_transport_closed()
                        }
                        None => break,
                    }
                };
                if closed {
                    let mut guard = this.service.lock().await;
                    if guard.take().is_some() {
                        this.inflight.lock().await.clear();
                        *this.capabilities.write().await = None;
                        this.set_state(ConnectionState::Lost {
                            reason: "连接已关闭（服务器退出或网络中断）".to_string(),
                            can_reconnect: true,
                        })
                        .await;
                        this.wire.push_system("连接丢失（服务器退出或网络中断）");
                        tracing::warn!(server_id = %this.server_id, "MCP 连接断开（lost）");
                    }
                    break;
                }
            }
        });
        if let Some(old) = self.monitor_task.lock().await.replace(task) {
            old.abort();
        }
    }

    /// 现代世代：订阅列表变更（tools / prompts / resources）
    pub async fn subscribe_list_changed(&self) -> Result<(), McpError> {
        let peer = self.peer().await?;
        let filter = SubscriptionFilter::builder()
            .tools_list_changed()
            .prompts_list_changed()
            .resources_list_changed()
            .build();
        // listen 会等待服务器确认；Subscription::next 拉取后续通知
        let mut subscription = peer
            .listen(filter)
            .await
            .map_err(|e| McpError::HandshakeFailed {
                message: format!("订阅列表变更失败：{e}"),
            })?;
        let hub = Arc::clone(&self.hub);
        let task = tokio::spawn(async move {
            while let Ok(Some(notification)) = subscription.next().await {
                let value = serde_json::to_value(&notification).unwrap_or(Value::Null);
                let method = value
                    .get("method")
                    .and_then(|m| m.as_str())
                    .unwrap_or("notifications/unknown")
                    .to_string();
                hub.push_notification(&method, value);
            }
        });
        if let Some(old) = self.subscription_task.lock().await.replace(task) {
            old.abort();
        }
        Ok(())
    }

    async fn peer(&self) -> Result<rmcp::service::Peer<RoleClient>, McpError> {
        let guard = self.service.lock().await;
        match guard.as_ref() {
            Some(service) => Ok(service.peer().clone()),
            None => Err(McpError::Internal {
                message: "连接不可用，请先连接服务器".to_string(),
            }),
        }
    }

    /// 能力快照（缓存优先；`refresh=true` 强制重拉）
    pub async fn capabilities(&self, refresh: bool) -> Result<CapabilitySnapshot, McpError> {
        if !refresh
            && let Some(snapshot) = self.capabilities.read().await.clone()
        {
            return Ok(snapshot);
        }
        match self.fetch_capabilities().await {
            Ok(snapshot) => {
                *self.capabilities.write().await = Some(snapshot.clone());
                Ok(snapshot)
            }
            Err(e) => {
                // 失败回退上次快照并标注过期
                let mut guard = self.capabilities.write().await;
                if let Some(previous) = guard.as_mut() {
                    previous.stale = true;
                    return Ok(previous.clone());
                }
                Err(e)
            }
        }
    }

    async fn fetch_capabilities(&self) -> Result<CapabilitySnapshot, McpError> {
        let peer = self.peer().await?;
        let timeout = Duration::from_secs(self.settings.request_timeout_secs.max(1));

        let result = tokio::time::timeout(timeout, async {
            let tools = peer.list_all_tools().await?;
            let resources = peer.list_all_resources().await?;
            let templates = peer.list_all_resource_templates().await?;
            let prompts = peer.list_all_prompts().await?;
            Ok::<_, ServiceError>((tools, resources, templates, prompts))
        })
        .await;

        match result {
            Err(_) => Err(McpError::Internal {
                message: "枚举接口超时".to_string(),
            }),
            Ok(Err(e)) => Err(McpError::Internal {
                message: format!("枚举接口失败：{e}"),
            }),
            Ok(Ok((tools, resources, templates, prompts))) => Ok(CapabilitySnapshot {
                server_id: self.server_id.clone(),
                tools: tools.iter().map(convert_tool).collect(),
                resources: resources.iter().map(convert_resource).collect(),
                resource_templates: templates.iter().map(convert_template).collect(),
                prompts: prompts.iter().map(convert_prompt).collect(),
                fetched_at: Utc::now(),
                stale: false,
            }),
        }
    }

    /// 健康探测：现代世代用幂等枚举探测，旧世代发送 ping 请求
    pub async fn probe(&self) -> Result<ProbeResult, McpError> {
        let peer = self.peer().await?;
        let era = match self.state().await {
            ConnectionState::Ready { handshake } => handshake.lifecycle,
            _ => LifecycleEra::Modern,
        };
        let timeout = Duration::from_secs(self.settings.request_timeout_secs.max(1));
        let started = Instant::now();

        let result = match era {
            LifecycleEra::Modern => {
                // 现代世代无 ping：用一次全量枚举作为幂等探测
                tokio::time::timeout(timeout, peer.list_all_tools())
                    .await
                    .map(|r| r.map(|tools| format!("可用（{} 个工具）", tools.len())))
            }
            LifecycleEra::Legacy => {
                let request = ClientRequest::PingRequest(PingRequest {
                    method: Default::default(),
                    extensions: Default::default(),
                });
                tokio::time::timeout(timeout, peer.send_request(request))
                    .await
                    .map(|r| r.map(|_| "可用（ping 成功）".to_string()))
            }
        };

        match result {
            Ok(Ok(detail)) => Ok(ProbeResult {
                ok: true,
                rtt_ms: started.elapsed().as_millis() as u64,
                detail,
            }),
            Ok(Err(e)) => Ok(ProbeResult {
                ok: false,
                rtt_ms: started.elapsed().as_millis() as u64,
                detail: format!("探测失败：{e}"),
            }),
            Err(_) => Ok(ProbeResult {
                ok: false,
                rtt_ms: started.elapsed().as_millis() as u64,
                detail: "探测超时".to_string(),
            }),
        }
    }

    /// 调用工具（低层请求：可控超时 / 可取消 / 进度归因 / 不自动驱动 MRTR）
    pub async fn call_tool(&self, name: &str, arguments: Value) -> CallRecord {
        self.call_tool_with_id(None, name, arguments).await
    }

    /// 调用工具（指定 call_id：UI 先拿到 id 再发起，便于"取消"按钮使用）
    pub async fn call_tool_with_id(
        &self,
        call_id: Option<String>,
        name: &str,
        arguments: Value,
    ) -> CallRecord {
        let record_params = serde_json::json!({ "name": name, "arguments": arguments.clone() });
        let params = match arguments {
            Value::Object(map) => CallToolRequestParams::new(name.to_string()).with_arguments(map),
            Value::Null => CallToolRequestParams::new(name.to_string()),
            other => {
                return self
                    .failed_record(
                        CallMethod::ToolsCall,
                        name,
                        other,
                        CallError::Validation {
                            message: "arguments 必须是 JSON 对象".to_string(),
                        },
                    )
                    .await;
            }
        };
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        self.invoke(
            CallMethod::ToolsCall,
            name.to_string(),
            request,
            record_params,
            call_id,
        )
        .await
    }

    /// 读取资源
    pub async fn read_resource(&self, uri: &str) -> CallRecord {
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
            ReadResourceRequestParams::new(uri.to_string()),
        ));
        self.invoke(
            CallMethod::ResourcesRead,
            uri.to_string(),
            request,
            serde_json::json!({ "uri": uri }),
            None,
        )
        .await
    }

    /// 获取提示词
    pub async fn get_prompt(&self, name: &str, arguments: Value) -> CallRecord {
        let mut params = GetPromptRequestParams::new(name.to_string());
        if let Value::Object(map) = arguments.clone() {
            params.arguments = Some(map);
        }
        let request = ClientRequest::GetPromptRequest(GetPromptRequest::new(params));
        self.invoke(
            CallMethod::PromptsGet,
            name.to_string(),
            request,
            serde_json::json!({ "name": name, "arguments": arguments }),
            None,
        )
        .await
    }

    /// 统一调用管线：超时 + 取消句柄 + 进度令牌 + MRTR 处理 + 记录组装
    async fn invoke(
        &self,
        method: CallMethod,
        target: String,
        request: ClientRequest,
        params: Value,
        call_id: Option<String>,
    ) -> CallRecord {
        let started = Instant::now();
        let call_id = call_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let timeout_secs = self.settings.request_timeout_secs.max(1);

        let peer = match self.peer().await {
            Ok(peer) => peer,
            Err(e) => {
                return self
                    .record_with_error(
                        method, target, params, started, None, CallError::TransportClosed {
                            reason: e.message(),
                        },
                    )
                    .await;
            }
        };
        let protocol_version = match self.state().await {
            ConnectionState::Ready { handshake } => handshake.protocol_version,
            _ => "unknown".to_string(),
        };

        let options = PeerRequestOptions::with_timeout(Duration::from_secs(timeout_secs)).with_meta(
            RequestMetaObject::with_progress_token(ProgressToken(
                rmcp::model::NumberOrString::String(std::sync::Arc::from(call_id.as_str())),
            )),
        );

        let handle = match peer.send_cancellable_request(request, options).await {
            Ok(handle) => handle,
            Err(e) => {
                let err = CallError::from_service_error(&e, timeout_secs);
                return self
                    .record_with_error(method, target, params, started, Some(protocol_version), err)
                    .await;
            }
        };

        // 响应等待在独立任务中进行：句柄所有权归该任务，取消通过"中止等待 + 发送取消通知"实现
        // （rmcp 3.5.1 的 RequestHandle::cancel 需要句柄所有权，跨任务取消无法直接调用；
        //  精确 requestId 关联待 rmcp 提供跨任务取消句柄后补齐，见 05 文档 M2 遗留项）
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let responder = tokio::spawn(async move {
            let result = handle.await_response().await;
            let _ = response_tx.send(result);
        });
        self.inflight.lock().await.insert(
            call_id.clone(),
            InFlight {
                abort: responder.abort_handle(),
            },
        );

        let response = match response_rx.await {
            Ok(result) => result,
            Err(_) => {
                // 等待任务被中止（取消）或异常结束
                return self
                    .record_with_error(
                        method,
                        target,
                        params,
                        started,
                        Some(protocol_version),
                        CallError::TransportClosed {
                            reason: "请求已取消".to_string(),
                        },
                    )
                    .await;
            }
        };
        self.inflight.lock().await.remove(&call_id);

        match response {
            Ok(ServerResult::CallToolResult(result)) => {
                let is_error = result.is_error.unwrap_or(false);
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                let error = if is_error {
                    Some(CallError::RpcError {
                        code: 0,
                        message: "工具返回 isError=true".to_string(),
                        data: Some(value.clone()),
                    })
                } else {
                    None
                };
                self.finish_record(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    Some(value),
                    error,
                    None,
                )
                .await
            }
            Ok(ServerResult::ReadResourceResult(result)) => {
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                self.finish_record(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    Some(value),
                    None,
                    None,
                )
                .await
            }
            Ok(ServerResult::GetPromptResult(result)) => {
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                self.finish_record(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    Some(value),
                    None,
                    None,
                )
                .await
            }
            Ok(ServerResult::InputRequiredResult(_)) => self
                .record_with_error(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    CallError::InputRequired,
                )
                .await,
            Ok(other) => {
                let value = serde_json::to_value(&other).unwrap_or(Value::Null);
                self.finish_record(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    Some(value),
                    None,
                    None,
                )
                .await
            }
            Err(e) => {
                let err = CallError::from_service_error(&e, timeout_secs);
                self.record_with_error(method, target, params, started, Some(protocol_version), err)
                    .await
            }
        }
    }

    /// 取消在途调用：中止本地等待 + 发送 `notifications/cancelled`（尽力而为）
    pub async fn cancel_call(&self, call_id: &str) -> bool {
        let entry = self.inflight.lock().await.remove(call_id);
        let Some(entry) = entry else { return false };
        entry.abort.abort();
        if let Ok(peer) = self.peer().await {
            let _ = peer
                .notify_cancelled(rmcp::model::CancelledNotificationParam::new(
                    None,
                    Some(format!("用户取消（call_id={call_id}）")),
                ))
                .await;
        }
        true
    }

    /// 健康探测记录（用于历史落库；耗时取探测 RTT）
    pub async fn record_probe(&self, result: &ProbeResult) -> CallRecord {
        let started = Instant::now();
        let error = if result.ok {
            None
        } else {
            Some(CallError::Internal {
                message: result.detail.clone(),
            })
        };
        let mut record = self
            .finish_record(
                CallMethod::Probe,
                "probe".to_string(),
                serde_json::json!({}),
                started,
                None,
                Some(serde_json::to_value(result).unwrap_or(Value::Null)),
                error,
                None,
            )
            .await;
        record.duration_ms = result.rtt_ms;
        record
    }

    async fn failed_record(
        &self,
        method: CallMethod,
        target: &str,
        params: Value,
        error: CallError,
    ) -> CallRecord {
        let started = Instant::now();
        self.record_with_error(method, target.to_string(), params, started, None, error)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_with_error(
        &self,
        method: CallMethod,
        target: String,
        params: Value,
        started: Instant,
        protocol_version: Option<String>,
        error: CallError,
    ) -> CallRecord {
        self.finish_record(
            method,
            target,
            params,
            started,
            protocol_version,
            None,
            Some(error),
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_record(
        &self,
        method: CallMethod,
        target: String,
        params: Value,
        started: Instant,
        protocol_version: Option<String>,
        result: Option<Value>,
        error: Option<CallError>,
        status_override: Option<CallStatus>,
    ) -> CallRecord {
        let size_bytes = result
            .as_ref()
            .and_then(|v| serde_json::to_vec(v).ok())
            .map(|b| b.len() as u64)
            .unwrap_or(0);
        let status = status_override.unwrap_or_else(|| {
            if error.is_some() {
                CallStatus::Error
            } else {
                CallStatus::Ok
            }
        });
        CallRecord {
            id: uuid::Uuid::new_v4().to_string(),
            server_id: self.server_id.clone(),
            server_name: self.server_name.clone(),
            method,
            target,
            params,
            result,
            error,
            status,
            duration_ms: started.elapsed().as_millis() as u64,
            protocol_version: protocol_version.unwrap_or_else(|| "unknown".to_string()),
            transport: self.transport_kind().await,
            truncated: false,
            size_bytes,
            created_at: Utc::now(),
        }
    }
}

/// 初始化错误 → 中文可读分类（v0.1.1：以 rmcp 官方 API 判定为主，文本匹配仅兜底；
/// v0.1.2：传输层错误细分为"连接被拒/超时/请求失败"，并净化 SDK 泛型类型噪音）
fn map_initialize_error(e: &rmcp::service::ClientInitializeError) -> McpError {
    use rmcp::service::ClientInitializeError;

    // 1) 认证失败：401/403（服务端要求授权）——不再依赖字符串猜测
    if e.is_authorization_required() {
        return McpError::Unauthorized {
            status: 401,
            detail: e.auth_challenge().map(ToString::to_string),
        };
    }

    match e {
        // 2) 版本不受支持：透传服务器支持列表
        ClientInitializeError::NoCompatibleProtocolVersion {
            server_supported, ..
        } => McpError::UnsupportedProtocolVersion {
            supported: server_supported.iter().map(ToString::to_string).collect(),
        },
        // 3) discover 与旧世代握手均失败：汇总两段原因便于排障
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            McpError::HandshakeFailed {
                message: format!(
                    "自动协商（server/discover）与旧世代握手（initialize）均失败：\
                     discover → {discover}；initialize → {fallback}"
                ),
            }
        }
        // 4) 传输层发送失败：细分连接被拒 / 超时 / 其他（v0.1.2）
        ClientInitializeError::TransportError { error, context } => {
            map_transport_error(error, context)
        }
        // 5) 兜底：保留原始文本（去除泛型噪音）；对常见文本形态做宽松分类
        other => {
            let text = strip_type_parameters(&other.to_string());
            let lower = text.to_lowercase();
            if lower.contains("401")
                || lower.contains("403")
                || lower.contains("unauthorized")
                || lower.contains("authorization required")
            {
                return McpError::Unauthorized {
                    status: 401,
                    detail: Some(text),
                };
            }
            if lower.contains("no compatible protocol version") {
                return McpError::UnsupportedProtocolVersion {
                    supported: Vec::new(),
                };
            }
            McpError::HandshakeFailed { message: text }
        }
    }
}

/// 传输层发送错误 → 中文定向提示（v0.1.2）
///
/// 三层策略：
/// 1. 结构化下探 `reqwest::Error`（连接被拒 / 超时 / 其他）；
/// 2. 文本兜底：reqwest 风格 `error sending request for url (<url>)` → 定向提示；
/// 3. 净化 SDK 泛型噪音后透传。
fn map_transport_error(
    error: &rmcp::transport::DynamicTransportError,
    context: &str,
) -> McpError {
    let stage = translate_transport_stage(context);

    if let Some(reqwest_error) = find_reqwest_error(error) {
        let url = reqwest_error
            .url()
            .map(ToString::to_string)
            .unwrap_or_else(|| "（未知地址）".to_string());
        let message = if reqwest_error.is_connect() {
            format!(
                "无法连接服务器 {url}（连接被拒绝或网络不可达）。\
                 请检查：URL 与端口是否正确、服务是否已启动、网络/代理设置。{stage}"
            )
        } else if reqwest_error.is_timeout() {
            format!("连接服务器超时：{url}。请检查网络与服务状态。{stage}")
        } else {
            format!("请求发送失败：{url}（{reqwest_error}）。{stage}")
        };
        return McpError::TransportFailed { message };
    }

    let text = strip_type_parameters(&error.to_string());
    if let Some(url) = extract_send_error_url(&text) {
        return McpError::TransportFailed {
            message: format!(
                "无法连接服务器 {url}（连接被拒绝、网络不可达或超时）。\
                 请检查：URL 与端口是否正确、服务是否已启动、网络/代理设置。{stage}"
            ),
        };
    }

    McpError::TransportFailed {
        message: format!("{text}（阶段：{context}）"),
    }
}

/// 下探 `reqwest::Error`：先按已知包装类型直接匹配，再沿 source 链查找
fn find_reqwest_error(
    error: &rmcp::transport::DynamicTransportError,
) -> Option<&reqwest::Error> {
    // Streamable HTTP 客户端的 `Client(E)` 变体不暴露 source，需直接下探
    if let Some(inner) = error.error.downcast_ref::<
        rmcp::transport::streamable_http_client::StreamableHttpError<reqwest::Error>,
    >()
        && let rmcp::transport::streamable_http_client::StreamableHttpError::Client(req) = inner
    {
        return Some(req);
    }

    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error.error.as_ref());
    while let Some(current) = source {
        if let Some(reqwest_error) = current.downcast_ref::<reqwest::Error>() {
            return Some(reqwest_error);
        }
        source = current.source();
    }
    None
}

/// 从 `error sending request for url (<url>)` 文本中提取 URL（文本兜底用）
fn extract_send_error_url(text: &str) -> Option<String> {
    const MARKER: &str = "error sending request for url (";
    let start = text.find(MARKER)? + MARKER.len();
    let end = text[start..].find(')')? + start;
    if end <= start {
        return None;
    }
    Some(text[start..end].to_string())
}

/// 传输阶段名 → 中文可读（保持简短）
fn translate_transport_stage(context: &str) -> &'static str {
    match context {
        "send discover request" => "阶段：协议协商（server/discover）",
        "send initialize request" => "阶段：握手（initialize）",
        "send initialized notification" => "阶段：握手完成通知",
        _ => "阶段：建立连接",
    }
}

/// 去除 Rust 泛型类型参数噪音（`WorkerTransport<...>` → `WorkerTransport`），
/// 仅用于展示文本（原始细节仍可通过日志/报文查看）。
fn strip_type_parameters(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.saturating_sub(1);
            }
            _ if depth == 0 => result.push(ch),
            _ => {}
        }
    }
    // 压缩多余空白
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 由 ServerPeerInfo 组装握手信息
fn build_handshake(info: Option<&ServerPeerInfo>) -> Handshake {
    match info {
        Some(info) => Handshake {
            server_name: info
                .server_info
                .as_ref()
                .map(|i| i.name.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            server_version: info
                .server_info
                .as_ref()
                .map(|i| i.version.to_string())
                .unwrap_or_default(),
            protocol_version: info.protocol_version.to_string(),
            lifecycle: if info.protocol_version.has_initialize() {
                LifecycleEra::Legacy
            } else {
                LifecycleEra::Modern
            },
            capabilities: serde_json::to_value(&info.capabilities).unwrap_or(Value::Null),
            instructions: info.instructions.clone(),
        },
        None => Handshake {
            server_name: "unknown".to_string(),
            server_version: String::new(),
            protocol_version: "unknown".to_string(),
            lifecycle: LifecycleEra::Legacy,
            capabilities: Value::Null,
            instructions: None,
        },
    }
}

/// rmcp Tool → ToolInfo
fn convert_tool(tool: &rmcp::model::Tool) -> ToolInfo {
    ToolInfo {
        name: tool.name.to_string(),
        title: tool.title.clone(),
        description: tool.description.as_ref().map(ToString::to_string),
        input_schema: Some(Value::Object(tool.input_schema.as_ref().clone())),
        output_schema: tool
            .output_schema
            .as_ref()
            .map(|s| Value::Object(s.as_ref().clone())),
        annotations: tool
            .annotations
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok()),
    }
}

/// rmcp Resource → ResourceInfo
fn convert_resource(resource: &rmcp::model::Resource) -> ResourceInfo {
    ResourceInfo {
        uri: resource.uri.clone(),
        name: resource.name.clone(),
        title: resource.title.clone(),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
        size: resource.size,
        annotations: resource
            .annotations
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok()),
    }
}

/// rmcp ResourceTemplate → ResourceTemplateInfo
fn convert_template(template: &rmcp::model::ResourceTemplate) -> ResourceTemplateInfo {
    ResourceTemplateInfo {
        uri_template: template.uri_template.clone(),
        name: template.name.clone(),
        title: template.title.clone(),
        description: template.description.clone(),
        mime_type: template.mime_type.clone(),
    }
}

/// rmcp Prompt → PromptInfo
fn convert_prompt(prompt: &rmcp::model::Prompt) -> PromptInfo {
    PromptInfo {
        name: prompt.name.clone(),
        title: prompt.title.clone(),
        description: prompt.description.clone(),
        arguments: prompt
            .arguments
            .as_ref()
            .map(|args| {
                args.iter()
                    .map(|arg| PromptArgument {
                        name: arg.name.clone(),
                        description: arg.description.clone(),
                        required: arg.required.unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_type_parameters_removes_generics() {
        let raw = "Send message error Transport [rmcp::transport::worker::WorkerTransport\
                   <rmcp::transport::streamable_http_client::StreamableHttpClientWorker\
                   <reqwest::async_impl::client::Client>>] error: Client error: \
                   error sending request for url (http://127.0.0.1/mcp), when send discover request";
        let cleaned = strip_type_parameters(raw);
        assert!(cleaned.contains("WorkerTransport"));
        assert!(!cleaned.contains("reqwest::async_impl"));
        assert!(!cleaned.contains('<'));
        assert!(cleaned.contains("error sending request for url (http://127.0.0.1/mcp)"));
    }

    #[test]
    fn strip_type_parameters_keeps_urls_with_generic_like_text() {
        let raw = "connect to https://example.com/mcp failed (code <abc>)";
        let cleaned = strip_type_parameters(raw);
        assert_eq!(cleaned, "connect to https://example.com/mcp failed (code )");
    }

    #[test]
    fn translate_stage_names() {
        assert_eq!(
            translate_transport_stage("send discover request"),
            "阶段：协议协商（server/discover）"
        );
        assert_eq!(
            translate_transport_stage("send initialize request"),
            "阶段：握手（initialize）"
        );
        assert_eq!(translate_transport_stage("other"), "阶段：建立连接");
    }

    #[test]
    fn transport_error_text_fallback_extracts_url_and_cleans() {
        // 模拟 rmcp 传输错误文本（含泛型噪音 + reqwest 发送失败描述）
        let err = rmcp::transport::DynamicTransportError::from_parts(
            "test-transport",
            std::any::TypeId::of::<()>(),
            Box::new(std::io::Error::other(
                "Client error: error sending request for url (http://127.0.0.1/mcp)",
            )),
        );
        let mapped = map_transport_error(&err, "send discover request");
        match mapped {
            McpError::TransportFailed { message } => {
                assert!(message.contains("无法连接服务器"), "应给出定向提示：{message}");
                assert!(message.contains("http://127.0.0.1/mcp"), "应包含目标地址：{message}");
                assert!(message.contains("server/discover"), "应包含阶段信息：{message}");
                assert!(!message.contains('<'), "不应包含泛型噪音：{message}");
            }
            other => panic!("期望 TransportFailed，实际：{other:?}"),
        }
    }

    #[test]
    fn extract_url_from_send_error() {
        let text = "Transport [x] error: Client error: error sending request for url (http://127.0.0.1/mcp), when send discover request";
        assert_eq!(
            extract_send_error_url(text),
            Some("http://127.0.0.1/mcp".to_string())
        );
        assert_eq!(extract_send_error_url("no url here"), None);
    }
}
