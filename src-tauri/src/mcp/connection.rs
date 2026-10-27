//! 单服务器连接：生命周期、能力枚举、调用管线（见设计文档 03 第 3/5 节）

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use rmcp::ClientServiceExt;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, GetPromptRequest,
    GetPromptRequestParams, PingRequest, ProgressToken, ReadResourceRequest,
    ReadResourceRequestParams, RequestMetaObject, ServerPeerInfo, ServerResult, SubscriptionFilter,
};
use rmcp::service::{
    ClientLifecycleMode, PeerRequestOptions, RoleClient, RunningService, ServiceError,
};
use rmcp::transport::IntoTransport;
use rmcp::transport::auth::{AuthError, AuthorizationManager, WWWAuthenticateParams};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use url::Url;

use super::error::{CallError, McpError};
use super::handler::{AppClientHandler, EventHub};
use super::models::{
    CallFrames, CallMethod, CallOrigin, CallRecord, CallStatus, CapabilitySnapshot,
    ConnectionState, Handshake, HttpConfig, LEGACY_FALLBACK_VERSION, LifecycleEra,
    PREFERRED_PROTOCOL_VERSIONS, ProbeResult, PromptArgument, PromptInfo, ResourceInfo,
    ResourceTemplateInfo, RuntimeSettings, StdioConfig, ToolInfo, TransportKind,
};
use super::oauth::AuthStateSink;
use super::transport;
use super::wire::{WireLog, WireTransport};

/// 在途调用条目（取消 = 中止本地等待 + 发送取消通知）
struct InFlight {
    abort: tokio::task::AbortHandle,
    canceled: Arc<std::sync::atomic::AtomicBool>,
}

/// 结构化 OAuth `WWW-Authenticate` 挑战（v0.3.1）
#[derive(Clone, Debug)]
pub struct OAuthChallenge {
    pub raw: String,
    pub error: Option<String>,
    pub required_scope: Option<String>,
    pub resource_metadata_url: Option<String>,
}

impl OAuthChallenge {
    pub fn from_header(raw: &str, base_url: Option<&Url>) -> Self {
        let fallback = Url::parse("http://localhost/").expect("固定 URL 解析失败");
        let params = WWWAuthenticateParams::parse(raw, base_url.unwrap_or(&fallback));
        Self {
            raw: raw.to_string(),
            error: params.error,
            required_scope: params.scope,
            resource_metadata_url: params.resource_metadata_url.map(|url| url.to_string()),
        }
    }

    pub fn is_insufficient_scope(&self) -> bool {
        self.error.as_deref() == Some("insufficient_scope")
    }
}

/// 单次调用选项（测试运行器使用独立 call_id / timeout / 来源）。
#[derive(Clone, Debug, Default)]
pub struct CallOptions {
    pub call_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub origin: CallOrigin,
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
    /// 当前连接是否使用 OAuth（决定 401 错误分类）
    oauth_enabled: AtomicBool,
    /// 当前 HTTP 资源 URL（用于解析相对 resource_metadata）
    http_url: Mutex<Option<Url>>,
    /// 最近一次 OAuth 挑战（登录命令读取）
    last_oauth_challenge: Mutex<Option<OAuthChallenge>>,
    /// 连接层认证失效回调（清 token cache / has_token）
    auth_state: tokio::sync::RwLock<Option<Arc<dyn AuthStateSink>>>,
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
            oauth_enabled: AtomicBool::new(false),
            http_url: Mutex::new(None),
            last_oauth_challenge: Mutex::new(None),
            auth_state: tokio::sync::RwLock::new(None),
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

    /// 注入连接层认证失效回调（由 ConnectionManager / 测试决定）
    pub async fn set_auth_state_sink(&self, sink: Arc<dyn AuthStateSink>) {
        *self.auth_state.write().await = Some(sink);
    }

    /// 最近一次 OAuth 挑战（401 或 403 step-up）
    pub async fn last_oauth_challenge(&self) -> Option<OAuthChallenge> {
        self.last_oauth_challenge.lock().await.clone()
    }

    fn oauth_enabled(&self) -> bool {
        self.oauth_enabled.load(Ordering::Relaxed)
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
        self.oauth_enabled.store(false, Ordering::Relaxed);
        *self.http_url.lock().await = None;
        *self.last_oauth_challenge.lock().await = None;
        *self.transport_kind.write().await = TransportKind::Stdio;
        let transport = transport::stdio::build_stdio(cfg, &self.hub)?;
        self.start(transport).await
    }

    /// 连接 Streamable HTTP 服务器
    ///
    /// OAuth 模式必须传入已初始化的 manager；无 OAuth 保持原 reqwest 路径。
    pub async fn connect_http(
        self: &Arc<Self>,
        cfg: &HttpConfig,
        auth_manager: Option<AuthorizationManager>,
    ) -> Result<(), McpError> {
        *self.transport_kind.write().await = TransportKind::StreamableHttp;
        *self.http_url.lock().await = Url::parse(&cfg.url).ok();
        if cfg.oauth.is_some() {
            let Some(manager) = auth_manager else {
                return Err(McpError::Config {
                    message: "OAuth 连接缺少认证管理器".to_string(),
                });
            };
            self.oauth_enabled.store(true, Ordering::Relaxed);
            let transport = transport::http::build_http_oauth(
                cfg,
                self.settings.request_timeout_secs,
                manager,
            )?;
            self.start(transport).await
        } else {
            self.oauth_enabled.store(false, Ordering::Relaxed);
            let transport = transport::http::build_http(cfg, self.settings.request_timeout_secs)?;
            self.start(transport).await
        }
    }

    /// 连接遗留 HTTP+SSE 服务器（规范已弃用，兼容旧服务器）
    pub async fn connect_sse(self: &Arc<Self>, cfg: &HttpConfig) -> Result<(), McpError> {
        if cfg.oauth.is_some() {
            return Err(McpError::Config {
                message: "OAuth 仅支持 Streamable HTTP，不支持遗留 SSE".to_string(),
            });
        }
        self.oauth_enabled.store(false, Ordering::Relaxed);
        *self.http_url.lock().await = None;
        *self.last_oauth_challenge.lock().await = None;
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
                self.set_state(ConnectionState::Error { error: err.clone() })
                    .await;
                return Err(err);
            }
            Ok(Err(e)) => {
                if let Some(challenge) = e.auth_challenge() {
                    let base = self.http_url.lock().await.clone();
                    *self.last_oauth_challenge.lock().await = Some(
                        OAuthChallenge::from_header(challenge, base.as_ref()),
                    );
                }
                let base = self.http_url.lock().await.clone();
                let err = map_initialize_error(&e, self.oauth_enabled(), base.as_ref());
                self.wire
                    .push_system(format!("连接失败：{}", err.message()));
                self.set_state(ConnectionState::Error { error: err.clone() })
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
        *self.last_oauth_challenge.lock().await = None;

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
            self.hub.push_notification(
                "mcp/notice",
                serde_json::json!({ "message": format!("列表变更订阅失败：{e}") }),
            );
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
        *self.last_oauth_challenge.lock().await = None;
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
                    let already_oauth_required = matches!(
                        this.state().await,
                        ConnectionState::Error {
                            error: McpError::OAuthRequired { .. }
                        }
                    );
                    let mut guard = this.service.lock().await;
                    if guard.take().is_some() {
                        this.inflight.lock().await.clear();
                        *this.capabilities.write().await = None;
                        if !already_oauth_required {
                            this.set_state(ConnectionState::Lost {
                                reason: "连接已关闭（服务器退出或网络中断）".to_string(),
                                can_reconnect: true,
                            })
                            .await;
                            this.wire.push_system("连接丢失（服务器退出或网络中断）");
                            tracing::warn!(server_id = %this.server_id, "MCP 连接断开（lost）");
                        }
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
        let server_caps = peer.peer_info().map(|info| info.capabilities.clone());
        let unknown_caps = server_caps.as_ref().is_none_or(|caps| {
            caps.tools.is_none() && caps.resources.is_none() && caps.prompts.is_none()
        });
        let mut builder = SubscriptionFilter::builder();
        if unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.tools.is_some())
        {
            builder = builder.tools_list_changed();
        }
        if unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.resources.is_some())
        {
            builder = builder.resources_list_changed();
        }
        if unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.prompts.is_some())
        {
            builder = builder.prompts_list_changed();
        }
        let filter = builder.build();
        // listen 会等待服务器确认；Subscription::next 拉取后续通知
        let mut subscription =
            peer.listen(filter)
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
        if !refresh && let Some(snapshot) = self.capabilities.read().await.clone() {
            return Ok(snapshot);
        }
        match self.fetch_capabilities().await {
            Ok(snapshot) => {
                *self.capabilities.write().await = Some(snapshot.clone());
                Ok(snapshot)
            }
            Err(e) => {
                self.wire
                    .push_system(format!("能力枚举失败：{}", e.message()));
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

    /// 按服务器声明的 capabilities 枚举，避免向旧/轻量服务器发送未实现的可选方法。
    ///
    /// 旧协议服务器常见只实现 `tools/list`，或声明了 `resources` 却未实现
    /// `resources/templates/list`。这些方法返回 `-32601` 时应降级为空列表并给出
    /// 警告，而不是让整个能力枚举失败。
    async fn fetch_capabilities(&self) -> Result<CapabilitySnapshot, McpError> {
        let peer = self.peer().await?;
        let base_url = self.http_url.lock().await.clone();
        let server_caps = peer.peer_info().map(|info| info.capabilities.clone());
        // 未声明任何已知能力（旧服务器或能力集缺失）时保守地探测全部列表，
        // 保持对非标准服务器的兼容性。
        let unknown_caps = server_caps.as_ref().is_none_or(|caps| {
            caps.tools.is_none() && caps.resources.is_none() && caps.prompts.is_none()
        });
        let tools_enabled = unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.tools.is_some());
        let resources_enabled = unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.resources.is_some());
        let prompts_enabled = unknown_caps
            || server_caps
                .as_ref()
                .is_some_and(|caps| caps.prompts.is_some());

        // 整个枚举共享一个超时预算，避免多个可选能力依次超时导致等待时间翻倍。
        let deadline =
            Instant::now() + Duration::from_secs(self.settings.request_timeout_secs.max(1));
        let mut warnings = Vec::new();
        let mut skipped = Vec::new();

        // tools/list 是核心能力：声明后若失败（-32601 除外）仍视为错误。
        let tools = if tools_enabled {
            let result =
                tokio::time::timeout(remaining_timeout(deadline), peer.list_all_tools()).await;
            if let Ok(Err(error)) = &result {
                self.maybe_mark_oauth_required(error).await;
            }
            let (items, warning) = handle_list_result(
                "tools/list",
                true,
                self.oauth_enabled(),
                base_url.as_ref(),
                result,
            )?;
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            items
        } else {
            skipped.push("tools/list（未声明 tools）");
            Vec::new()
        };

        // resources/prompts 及可选的模板列表属于增强能力：失败降级为警告，
        // 避免一个未实现的可选方法拖垮工具列表。
        let (resources, templates) = if resources_enabled {
            let result =
                tokio::time::timeout(remaining_timeout(deadline), peer.list_all_resources()).await;
            if let Ok(Err(error)) = &result {
                self.maybe_mark_oauth_required(error).await;
            }
            let (resources, warning) = handle_list_result(
                "resources/list",
                false,
                self.oauth_enabled(),
                base_url.as_ref(),
                result,
            )?;
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            let result = tokio::time::timeout(
                remaining_timeout(deadline),
                peer.list_all_resource_templates(),
            )
            .await;
            if let Ok(Err(error)) = &result {
                self.maybe_mark_oauth_required(error).await;
            }
            let (templates, warning) = handle_list_result(
                "resources/templates/list",
                false,
                self.oauth_enabled(),
                base_url.as_ref(),
                result,
            )?;
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            (resources, templates)
        } else {
            skipped.push("resources/list、resources/templates/list（未声明 resources）");
            (Vec::new(), Vec::new())
        };

        let prompts = if prompts_enabled {
            let result =
                tokio::time::timeout(remaining_timeout(deadline), peer.list_all_prompts()).await;
            if let Ok(Err(error)) = &result {
                self.maybe_mark_oauth_required(error).await;
            }
            let (items, warning) = handle_list_result(
                "prompts/list",
                false,
                self.oauth_enabled(),
                base_url.as_ref(),
                result,
            )?;
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            items
        } else {
            skipped.push("prompts/list（未声明 prompts）");
            Vec::new()
        };

        if !skipped.is_empty() {
            self.wire
                .push_system(format!("按服务器能力跳过枚举：{}", skipped.join("、")));
        }
        for warning in &warnings {
            self.wire.push_system(format!("能力枚举降级：{warning}"));
        }

        Ok(CapabilitySnapshot {
            server_id: self.server_id.clone(),
            tools: tools.iter().map(convert_tool).collect(),
            resources: resources.iter().map(convert_resource).collect(),
            resource_templates: templates.iter().map(convert_template).collect(),
            prompts: prompts.iter().map(convert_prompt).collect(),
            fetched_at: Utc::now(),
            stale: false,
            warnings,
        })
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
                // 现代世代无 ping：按服务器声明的能力选择一次幂等枚举作为探测。
                let server_caps = peer.peer_info().map(|info| info.capabilities.clone());
                tokio::time::timeout(timeout, async {
                    match server_caps.as_ref() {
                        Some(caps)
                            if caps.tools.is_some()
                                || (caps.resources.is_none() && caps.prompts.is_none()) =>
                        {
                            peer.list_all_tools()
                                .await
                                .map(|tools| format!("可用（{} 个工具）", tools.len()))
                        }
                        Some(caps) if caps.resources.is_some() => peer
                            .list_all_resources()
                            .await
                            .map(|items| format!("可用（{} 个资源）", items.len())),
                        Some(caps) if caps.prompts.is_some() => peer
                            .list_all_prompts()
                            .await
                            .map(|items| format!("可用（{} 个提示词）", items.len())),
                        Some(_) => Ok("可用（服务器未声明可枚举能力）".to_string()),
                        None => peer
                            .list_all_tools()
                            .await
                            .map(|tools| format!("可用（{} 个工具）", tools.len())),
                    }
                })
                .await
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
            Ok(Err(e)) => {
                self.maybe_mark_oauth_required(&e).await;
                Ok(ProbeResult {
                    ok: false,
                    rtt_ms: started.elapsed().as_millis() as u64,
                    detail: format!("探测失败：{e}"),
                })
            }
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
        self.call_tool_with_options(
            name,
            arguments,
            CallOptions {
                call_id,
                ..Default::default()
            },
        )
        .await
    }

    /// 调用工具（测试运行器：独立 call_id / timeout / origin）。
    pub async fn call_tool_with_options(
        &self,
        name: &str,
        arguments: Value,
        options: CallOptions,
    ) -> CallRecord {
        let record_params = serde_json::json!({ "name": name, "arguments": arguments.clone() });
        let params = match arguments {
            Value::Object(map) => CallToolRequestParams::new(name.to_string()).with_arguments(map),
            Value::Null => CallToolRequestParams::new(name.to_string()),
            other => {
                let mut record = self
                    .failed_record(
                        CallMethod::ToolsCall,
                        name,
                        other,
                        CallError::Validation {
                            message: "arguments 必须是 JSON 对象".to_string(),
                        },
                    )
                    .await;
                if let Some(call_id) = options.call_id {
                    record.id = call_id;
                }
                record.origin = options.origin;
                return record;
            }
        };
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        self.invoke(
            CallMethod::ToolsCall,
            name.to_string(),
            request,
            record_params,
            options,
        )
        .await
    }

    /// 读取资源
    pub async fn read_resource(&self, uri: &str) -> CallRecord {
        self.read_resource_with_options(uri, CallOptions::default())
            .await
    }

    pub async fn read_resource_with_options(&self, uri: &str, options: CallOptions) -> CallRecord {
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
            ReadResourceRequestParams::new(uri.to_string()),
        ));
        self.invoke(
            CallMethod::ResourcesRead,
            uri.to_string(),
            request,
            serde_json::json!({ "uri": uri }),
            options,
        )
        .await
    }

    /// 获取提示词
    pub async fn get_prompt(&self, name: &str, arguments: Value) -> CallRecord {
        self.get_prompt_with_options(name, arguments, CallOptions::default())
            .await
    }

    pub async fn get_prompt_with_options(
        &self,
        name: &str,
        arguments: Value,
        options: CallOptions,
    ) -> CallRecord {
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
            options,
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
        options: CallOptions,
    ) -> CallRecord {
        let call_id = options
            .call_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut options = options;
        options.call_id = Some(call_id.clone());
        let origin = options.origin.clone();
        let mut record = self
            .invoke_inner(method, target, request, params, options)
            .await;
        record.id = call_id.clone();

        let mut frames = self.wire.take_capture(&call_id).map(|capture| CallFrames {
            rpc_id: capture.rpc_id,
            request: capture.request,
            response: capture.response,
            truncated: capture.truncated,
        });
        if let Some(frames) = frames.as_mut() {
            let drop_response = record.status == CallStatus::Canceled
                || (record.result.is_none()
                    && matches!(
                        record.error.as_ref(),
                        Some(CallError::Timeout { .. } | CallError::TransportClosed { .. })
                    ));
            if drop_response && frames.response.is_some() {
                frames.response = None;
                frames.truncated = true;
            }
        }
        if frames
            .as_ref()
            .is_some_and(|item| item.request.is_none() && item.response.is_none())
        {
            frames = None;
        }
        record.frames = frames;
        record.origin = origin;
        record
    }

    async fn invoke_inner(
        &self,
        method: CallMethod,
        target: String,
        request: ClientRequest,
        params: Value,
        options: CallOptions,
    ) -> CallRecord {
        let started = Instant::now();
        let call_id = options
            .call_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let timeout_secs = options
            .timeout_secs
            .unwrap_or(self.settings.request_timeout_secs)
            .max(1);

        let peer = match self.peer().await {
            Ok(peer) => peer,
            Err(e) => {
                return self
                    .record_with_error(
                        method,
                        target,
                        params,
                        started,
                        None,
                        CallError::TransportClosed {
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

        let options = PeerRequestOptions::with_timeout(Duration::from_secs(timeout_secs))
            .with_meta(RequestMetaObject::with_progress_token(ProgressToken(
                rmcp::model::NumberOrString::String(std::sync::Arc::from(call_id.as_str())),
            )));

        let handle = match peer.send_cancellable_request(request, options).await {
            Ok(handle) => handle,
            Err(e) => {
                self.maybe_mark_oauth_required(&e).await;
                let base_url = self.http_url.lock().await.clone();
                let err = CallError::from_service_error(
                    &e,
                    timeout_secs,
                    self.oauth_enabled(),
                    base_url.as_ref(),
                );
                return self
                    .record_with_error(method, target, params, started, Some(protocol_version), err)
                    .await;
            }
        };

        // 按 rpc_id 把原始帧归属到本次 call_id
        self.wire.link_capture(&handle.id, &call_id);

        // 响应等待在独立任务中进行：句柄所有权归该任务，取消通过"中止等待 + 发送取消通知"实现
        // （rmcp 3.5.1 的 RequestHandle::cancel 需要句柄所有权，跨任务取消无法直接调用；
        //  精确 requestId 关联待 rmcp 提供跨任务取消句柄后补齐，见 05 文档 M2 遗留项）
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let responder = tokio::spawn(async move {
            let result = handle.await_response().await;
            let _ = response_tx.send(result);
        });
        let canceled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.inflight.lock().await.insert(
            call_id.clone(),
            InFlight {
                abort: responder.abort_handle(),
                canceled: Arc::clone(&canceled),
            },
        );

        let response = match response_rx.await {
            Ok(result) => result,
            Err(_) => {
                // 等待任务被中止（取消）或异常结束
                if canceled.load(std::sync::atomic::Ordering::SeqCst) {
                    return self
                        .finish_record(
                            method,
                            target,
                            params,
                            started,
                            Some(protocol_version),
                            None,
                            None,
                            Some(CallStatus::Canceled),
                        )
                        .await;
                }
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
            Ok(ServerResult::InputRequiredResult(_)) => {
                self.record_with_error(
                    method,
                    target,
                    params,
                    started,
                    Some(protocol_version),
                    CallError::InputRequired,
                )
                .await
            }
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
                self.maybe_mark_oauth_required(&e).await;
                let base_url = self.http_url.lock().await.clone();
                let err = CallError::from_service_error(
                    &e,
                    timeout_secs,
                    self.oauth_enabled(),
                    base_url.as_ref(),
                );
                self.record_with_error(method, target, params, started, Some(protocol_version), err)
                    .await
            }
        }
    }

    /// 401/refresh 失效：置连接状态、清能力快照，并通知 OAuth 服务清 cache
    async fn mark_oauth_required(&self, challenge: OAuthChallenge) {
        *self.last_oauth_challenge.lock().await = Some(challenge.clone());
        *self.capabilities.write().await = None;
        self.set_state(ConnectionState::Error {
            error: McpError::OAuthRequired {
                detail: Some(challenge.raw),
                resource_metadata_url: challenge.resource_metadata_url,
                required_scope: challenge.required_scope,
            },
        })
        .await;
        if let Some(sink) = self.auth_state.read().await.clone() {
            sink.on_auth_required(&self.server_id).await;
        }
    }

    /// 调用/probe/能力枚举失败时检查认证失败类型
    async fn maybe_mark_oauth_required(&self, error: &ServiceError) {
        if !self.oauth_enabled() {
            return;
        }
        let base = self.http_url.lock().await.clone();
        let failure = match error {
            ServiceError::TransportSend(error) => {
                classify_auth_failure(error.error.as_ref(), true, base.as_ref())
            }
            _ => None,
        };
        match failure {
            Some(AuthFailure::Unauthorized(challenge)) => {
                self.mark_oauth_required(challenge).await;
            }
            Some(AuthFailure::InsufficientScope(challenge)) => {
                // 403 是调用级权限不足：保留连接，只记录挑战供“补充授权”
                *self.last_oauth_challenge.lock().await = Some(challenge);
            }
            Some(AuthFailure::Forbidden { .. } | AuthFailure::Transient(_)) | None => {}
        }
    }

    /// 取消在途调用：中止本地等待 + 发送 `notifications/cancelled`（尽力而为）
    pub async fn cancel_call(&self, call_id: &str) -> bool {
        let entry = self.inflight.lock().await.remove(call_id);
        let Some(entry) = entry else { return false };
        entry
            .canceled
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
            origin: CallOrigin::Manual,
            frames: None,
            created_at: Utc::now(),
        }
    }
}

/// 处理单个列表枚举结果：
/// - 传输关闭/取消：始终视为致命错误；
/// - `-32601`：服务器未实现该方法，降级为空列表；
/// - 非核心能力（`fatal=false`）的其他错误或超时：降级为空列表并给出警告；
/// - 核心能力（`fatal=true`）的其他错误或超时：返回错误。
fn handle_list_result<T>(
    label: &str,
    fatal: bool,
    oauth_enabled: bool,
    base_url: Option<&Url>,
    result: Result<Result<Vec<T>, ServiceError>, tokio::time::error::Elapsed>,
) -> Result<(Vec<T>, Option<String>), McpError> {
    if let Ok(Err(error)) = &result
        && oauth_enabled
        && let Some(failure) = classify_auth_failure(error, true, base_url)
    {
        return match failure {
            AuthFailure::Unauthorized(challenge) | AuthFailure::InsufficientScope(challenge) => {
                Err(McpError::OAuthRequired {
                    detail: Some(challenge.raw),
                    resource_metadata_url: challenge.resource_metadata_url,
                    required_scope: challenge.required_scope,
                })
            }
            AuthFailure::Forbidden { status, detail } if fatal => Err(McpError::Unauthorized {
                status,
                detail: Some(detail),
            }),
            AuthFailure::Forbidden { .. } => Ok((
                Vec::new(),
                Some(format!("{label} 返回 403（非 step-up），已跳过")),
            )),
            AuthFailure::Transient(message) if !fatal => Ok((
                Vec::new(),
                Some(format!("{label} OAuth 临时认证失败：{message}，已跳过")),
            )),
            AuthFailure::Transient(message) => Err(McpError::TransportFailed {
                message: format!("OAuth 临时认证失败：{message}"),
            }),
        };
    }
    match result {
        Ok(Ok(items)) => Ok((items, None)),
        Ok(Err(error)) if is_connection_fatal(&error) => Err(McpError::Internal {
            message: format!("枚举 {label} 失败：{error}"),
        }),
        Ok(Err(error)) if is_method_not_found(&error) => Ok((
            Vec::new(),
            Some(format!(
                "{label} 返回 -32601（Method not found），已按空列表处理"
            )),
        )),
        Ok(Err(error)) if !fatal => Ok((
            Vec::new(),
            Some(format!("{label} 枚举失败：{error}，已按空列表处理")),
        )),
        Ok(Err(error)) => Err(McpError::Internal {
            message: format!("枚举 {label} 失败：{error}"),
        }),
        Err(_) if !fatal => Ok((Vec::new(), Some(format!("{label} 枚举超时，已跳过")))),
        Err(_) => Err(McpError::Internal {
            message: format!("枚举 {label} 超时"),
        }),
    }
}

/// 共享超时预算的剩余时间（至少 1ms，避免已耗尽时立即误判）。
fn remaining_timeout(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1))
}

/// 传输层错误：连接已不可用，继续枚举其他能力没有意义。
fn is_connection_fatal(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::TransportClosed
            | ServiceError::TransportSend(_)
            | ServiceError::Cancelled { .. }
    )
}

/// JSON-RPC `-32601 Method not found`（服务器未实现该方法）。
fn is_method_not_found(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::McpError(data)
            if data.code == rmcp::model::ErrorCode::METHOD_NOT_FOUND
    )
}

/// 初始化错误 → 中文可读分类（v0.3.0：OAuth 与非 OAuth 401 分离）
fn map_initialize_error(
    e: &rmcp::service::ClientInitializeError,
    oauth_enabled: bool,
    base_url: Option<&Url>,
) -> McpError {
    use rmcp::service::ClientInitializeError;

    // 1) 认证失败：401/403（服务端要求授权）
    if e.is_authorization_required() {
        let detail = e.auth_challenge().map(ToString::to_string);
        let challenge = detail
            .as_deref()
            .map(|raw| OAuthChallenge::from_header(raw, base_url));
        if oauth_enabled {
            if let Some(challenge) = challenge {
                return McpError::OAuthRequired {
                    detail: Some(challenge.raw),
                    resource_metadata_url: challenge.resource_metadata_url,
                    required_scope: challenge.required_scope,
                };
            }
            return McpError::OAuthRequired {
                detail,
                resource_metadata_url: None,
                required_scope: None,
            };
        }
        let status = if challenge
            .as_ref()
            .is_some_and(OAuthChallenge::is_insufficient_scope)
        {
            403
        } else {
            401
        };
        return McpError::Unauthorized { status, detail };
    }

    // 2) 初始化阶段已经暴露 AuthError（如刷新失败）时也按 OAuth 分类
    if oauth_enabled && let Some(auth_error) = find_auth_error(e) {
        return match auth_error {
            AuthError::TokenRefreshFailed(message) => McpError::TransportFailed {
                message: format!("OAuth token 刷新失败：{message}"),
            },
            AuthError::InsufficientScope {
                required_scope, ..
            } => McpError::OAuthRequired {
                detail: Some(format!(
                    "Bearer error=\"insufficient_scope\", scope=\"{required_scope}\""
                )),
                resource_metadata_url: None,
                required_scope: Some(required_scope.clone()),
            },
            _ => McpError::OAuthRequired {
                detail: Some(strip_type_parameters(&auth_error.to_string())),
                resource_metadata_url: None,
                required_scope: None,
            },
        };
    }

    match e {
        // 3) 版本不受支持：透传服务器支持列表
        ClientInitializeError::NoCompatibleProtocolVersion {
            server_supported, ..
        } => McpError::UnsupportedProtocolVersion {
            supported: server_supported.iter().map(ToString::to_string).collect(),
        },
        // 4) discover 与旧世代握手均失败：汇总两段原因便于排障
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            McpError::HandshakeFailed {
                message: format!(
                    "自动协商（server/discover）与旧世代握手（initialize）均失败：\
                     discover → {discover}；initialize → {fallback}"
                ),
            }
        }
        // 5) 传输层发送失败：细分连接被拒 / 超时 / 其他
        ClientInitializeError::TransportError { error, context } => {
            if oauth_enabled && find_auth_error_in(error.error.as_ref()).is_some() {
                return McpError::OAuthRequired {
                    detail: Some(strip_type_parameters(&error.to_string())),
                    resource_metadata_url: None,
                    required_scope: None,
                };
            }
            map_transport_error(error, context)
        }
        // 6) 兜底：保留原始文本（去除泛型噪音）；对常见文本形态做宽松分类
        other => {
            let text = strip_type_parameters(&other.to_string());
            let lower = text.to_lowercase();
            if lower.contains("401")
                || lower.contains("403")
                || lower.contains("unauthorized")
                || lower.contains("authorization required")
            {
                let challenge = OAuthChallenge::from_header(&text, base_url);
                if oauth_enabled {
                    return McpError::OAuthRequired {
                        detail: Some(challenge.raw),
                        resource_metadata_url: challenge.resource_metadata_url,
                        required_scope: challenge.required_scope,
                    };
                }
                return McpError::Unauthorized {
                    status: if challenge.is_insufficient_scope() {
                        403
                    } else {
                        401
                    },
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

/// 在初始化错误 source 链中查找 rmcp AuthError
fn find_auth_error(e: &rmcp::service::ClientInitializeError) -> Option<&AuthError> {
    use rmcp::service::ClientInitializeError;
    match e {
        ClientInitializeError::TransportError { error, .. } => {
            find_auth_error_in(error.error.as_ref())
        }
        ClientInitializeError::LegacyFallbackFailed { fallback, .. } => find_auth_error(fallback),
        _ => None,
    }
}

/// 沿 source 链查找认证错误
fn find_auth_error_in<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a AuthError> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(auth) = current.downcast_ref::<AuthError>() {
            return Some(auth);
        }
        source = current.source();
    }
    None
}

/// 供 `CallError::from_service_error` 复用（OAuth 已启用时）
pub(crate) fn classify_oauth_service_error(
    error: &rmcp::transport::DynamicTransportError,
    base_url: Option<&Url>,
) -> Option<AuthFailure> {
    classify_auth_failure(error.error.as_ref(), true, base_url)
}

/// 连接已建立后的认证失败分类（只在 OAuth 模式下返回 Some）
#[derive(Debug)]
pub(crate) enum AuthFailure {
    Unauthorized(OAuthChallenge),
    InsufficientScope(OAuthChallenge),
    Forbidden { status: u16, detail: String },
    Transient(String),
}

fn classify_auth_failure(
    error: &(dyn std::error::Error + 'static),
    oauth_enabled: bool,
    base_url: Option<&Url>,
) -> Option<AuthFailure> {
    if !oauth_enabled {
        return None;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(auth) = current.downcast_ref::<AuthError>() {
            return match auth {
                AuthError::AuthorizationRequired | AuthError::TokenRefreshRejected(_) => Some(
                    AuthFailure::Unauthorized(OAuthChallenge::from_header(
                        &strip_type_parameters(&auth.to_string()),
                        base_url,
                    )),
                ),
                AuthError::InsufficientScope {
                    required_scope, ..
                } => Some(AuthFailure::InsufficientScope(OAuthChallenge {
                    raw: format!(
                        "Bearer error=\"insufficient_scope\", scope=\"{required_scope}\""
                    ),
                    error: Some("insufficient_scope".to_string()),
                    required_scope: Some(required_scope.clone()),
                    resource_metadata_url: None,
                })),
                AuthError::TokenRefreshFailed(message) => {
                    Some(AuthFailure::Transient(message.clone()))
                }
                AuthError::CredentialStoreError(message) => {
                    Some(AuthFailure::Transient(message.clone()))
                }
                _ => None,
            };
        }
        if let Some(required) =
            current.downcast_ref::<rmcp::transport::streamable_http_client::AuthRequiredError>()
        {
            return Some(AuthFailure::Unauthorized(OAuthChallenge::from_header(
                &required.www_authenticate_header,
                base_url,
            )));
        }
        if let Some(scope) = current
            .downcast_ref::<rmcp::transport::streamable_http_client::InsufficientScopeError>()
        {
            let challenge =
                OAuthChallenge::from_header(&scope.www_authenticate_header, base_url);
            return if challenge.is_insufficient_scope() {
                Some(AuthFailure::InsufficientScope(challenge))
            } else {
                Some(AuthFailure::Forbidden {
                    status: 403,
                    detail: scope.www_authenticate_header.clone(),
                })
            };
        }
        if let Some(reqwest) = current.downcast_ref::<reqwest::Error>()
            && let Some(status) = reqwest.status()
            && status == reqwest::StatusCode::FORBIDDEN
        {
            return Some(AuthFailure::Forbidden {
                status: 403,
                detail: strip_type_parameters(&reqwest.to_string()),
            });
        }
        source = current.source();
    }
    None
}

/// 传输层发送错误 → 中文定向提示（v0.1.2）
///
/// 三层策略：
/// 1. 结构化下探 `reqwest::Error`（连接被拒 / 超时 / 其他）；
/// 2. 文本兜底：reqwest 风格 `error sending request for url (<url>)` → 定向提示；
/// 3. 净化 SDK 泛型噪音后透传。
fn map_transport_error(error: &rmcp::transport::DynamicTransportError, context: &str) -> McpError {
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
fn find_reqwest_error(error: &rmcp::transport::DynamicTransportError) -> Option<&reqwest::Error> {
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
    fn classify_auth_error_requires_oauth_flag() {
        let error = rmcp::transport::streamable_http_client::AuthRequiredError::new(
            "Bearer error=\"invalid_token\"".to_string(),
        );
        assert!(
            classify_auth_failure(&error, true, None).is_some(),
            "OAuth 模式应识别 401"
        );
        assert!(
            classify_auth_failure(&error, false, None).is_none(),
            "非 OAuth 模式不得归类为 OAuthRequired"
        );
    }

    #[test]
    fn classify_insufficient_scope_and_forbidden() {
        let step = rmcp::transport::streamable_http_client::InsufficientScopeError::new(
            "Bearer error=\"insufficient_scope\", scope=\"write\"".to_string(),
            Some("write".to_string()),
        );
        assert!(matches!(
            classify_auth_failure(&step, true, None),
            Some(AuthFailure::InsufficientScope(challenge))
                if challenge.required_scope.as_deref() == Some("write")
        ));

        let plain =
            rmcp::transport::streamable_http_client::InsufficientScopeError::new(
                "Bearer realm=\"mcp\"".to_string(),
                None,
            );
        assert!(matches!(
            classify_auth_failure(&plain, true, None),
            Some(AuthFailure::Forbidden { status: 403, .. })
        ));
    }

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
                assert!(
                    message.contains("无法连接服务器"),
                    "应给出定向提示：{message}"
                );
                assert!(
                    message.contains("http://127.0.0.1/mcp"),
                    "应包含目标地址：{message}"
                );
                assert!(
                    message.contains("server/discover"),
                    "应包含阶段信息：{message}"
                );
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
