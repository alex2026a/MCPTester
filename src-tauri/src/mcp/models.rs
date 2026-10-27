//! MCP 领域模型与前端契约（见设计文档 04 第 1 节）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 传输类型
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Stdio,
    StreamableHttp,
    /// 遗留 HTTP+SSE（规范已弃用，兼容实现）
    Sse,
}

impl TransportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::Stdio => "stdio",
            TransportKind::StreamableHttp => "streamable_http",
            TransportKind::Sse => "sse",
        }
    }
}

/// 键值项（env 变量 / HTTP 头）
///
/// 持久化时 `secret=true` 的值存 keyring（版本化键名），SQLite 不存明文；
/// 运行期（connect 前）由上层把 keyring 中的值合并回 `value`。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KeyValue {
    pub key: String,
    pub value: Option<String>,
    pub secret: bool,
}

/// Windows 命令解析模式（见 03 文档 4.3）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellWrap {
    /// 默认：which_command 解析全路径后原样 spawn
    #[default]
    Resolve,
    /// 显式 cmd /C 包装（参数转义与进程树风险，UI 提示）
    CmdShell,
    /// 原样 spawn（排障用）
    Never,
}

/// stdio 传输配置
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StdioConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<KeyValue>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub shell_wrap: ShellWrap,
}

/// OAuth 授权请求额外参数（v0.3.1）
///
/// 只追加到 authorization request，不进入 token/refresh 请求；key 由后端
/// 校验，禁止覆盖 state/PKCE/client_id 等保留参数。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct OAuthParam {
    pub key: String,
    pub value: String,
}

/// OAuth 2.1 登录配置（v0.3.0；v0.3.1 增加互操作字段）
///
/// `Some/None` 即开关；`client_secret` 只用于 UI 回显掩码，真实值保存在
/// `oauth_credentials`。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct OAuthConfig {
    /// 请求的 scope；空 = 由授权服务器元数据自动选择
    #[serde(default)]
    pub scopes: Vec<String>,
    /// 预注册 client_id；空 = 动态客户端注册
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// UI 掩码；真实 secret 由 `ServerDraft.secrets` 写入 oauth_credentials
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// 完整回调地址；None = http://127.0.0.1:33318/oauth/callback
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    /// 手工端点模式的 issuer（可选；用于 RFC 9207 校验）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// 容忍元数据缺少 issuer（仅兼容不规范服务器）
    #[serde(default)]
    pub allow_missing_issuer: bool,
    /// 授权请求额外参数（v0.3.1）；只追加到 authorization request
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorize_params: Vec<OAuthParam>,
    /// 显式允许把自定义 header 发往跨源授权服务器（v0.3.1）
    #[serde(default)]
    pub forward_headers_to_auth_server: bool,
    /// CIMD（SEP-991）客户端元数据文档 URL；None = 不启用
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_metadata_url: Option<String>,
    /// 手工端点模式的 token 端点认证方式
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    /// 退出登录时使用的撤销端点（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
}

/// HTTP / SSE 传输配置
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HttpConfig {
    pub url: String,
    pub headers: Vec<KeyValue>,
    /// 服务器级超时覆盖；None = 沿用全局设置
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub allow_insecure_tls: bool,
    /// Some = 使用 OAuth；None = 手工 Authorization / 无认证
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
}

/// 服务器来源（配置文件 seed 的服务器带此标记）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ServerOrigin {
    /// 来源类型：目前为 "config"
    pub source: String,
    /// 配置文件中的稳定 key
    pub key: String,
}

/// 服务器配置（持久化于 servers.config_json）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub transport: TransportKind,
    pub stdio: Option<StdioConfig>,
    pub http: Option<HttpConfig>,
    pub protocol_version_pin: Option<String>,
    pub enabled: bool,
    pub tags: Vec<String>,
    /// 配置文件来源；手工添加的服务器为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ServerOrigin>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_connected_at: Option<DateTime<Utc>>,
}

/// 生命周期世代（协商来源）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEra {
    /// 2026-07-28：discover 协商
    Modern,
    /// 2025-11-25 及更早：initialize 握手
    Legacy,
}

/// 握手信息（Ready 时可用）
#[derive(Serialize, Clone, Debug)]
pub struct Handshake {
    pub server_name: String,
    pub server_version: String,
    /// 协商结果（如 "2026-07-28" / "2025-06-18"）
    pub protocol_version: String,
    pub lifecycle: LifecycleEra,
    /// 服务器能力集原样透传
    pub capabilities: serde_json::Value,
    /// 服务器 instructions 原文（现代世代可能缺省）
    pub instructions: Option<String>,
}

/// 连接状态（事件 `mcp://connection-state` 载荷）
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Ready { handshake: Handshake },
    Degraded { reason: String },
    Lost { reason: String, can_reconnect: bool },
    Error { error: super::error::McpError },
}

impl ConnectionState {
    /// 是否处于可调用状态
    pub fn is_ready(&self) -> bool {
        matches!(
            self,
            ConnectionState::Ready { .. } | ConnectionState::Degraded { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConnectionState::Disconnected => "disconnected",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Ready { .. } => "ready",
            ConnectionState::Degraded { .. } => "degraded",
            ConnectionState::Lost { .. } => "lost",
            ConnectionState::Error { .. } => "error",
        }
    }
}

/// 工具信息（input_schema 原样透传，前端据此生成表单）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
    pub annotations: Option<serde_json::Value>,
}

/// 资源信息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceInfo {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<u64>,
    pub annotations: Option<serde_json::Value>,
}

/// 资源模板信息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceTemplateInfo {
    pub uri_template: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// 提示词参数
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

/// 提示词信息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PromptInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub arguments: Vec<PromptArgument>,
}

/// 能力快照（仅内存缓存）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CapabilitySnapshot {
    pub server_id: String,
    pub tools: Vec<ToolInfo>,
    pub resources: Vec<ResourceInfo>,
    pub resource_templates: Vec<ResourceTemplateInfo>,
    pub prompts: Vec<PromptInfo>,
    pub fetched_at: DateTime<Utc>,
    /// 是否因刷新失败而沿用旧快照
    pub stale: bool,
    /// 枚举降级提示（如服务器声明了某能力但方法未实现）
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// 调用方法（历史记录 method 字段）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallMethod {
    ToolsCall,
    ResourcesRead,
    PromptsGet,
    /// 健康探测（现代世代无 ping；旧世代用 ping 请求）
    Probe,
}

impl CallMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallMethod::ToolsCall => "tools_call",
            CallMethod::ResourcesRead => "resources_read",
            CallMethod::PromptsGet => "prompts_get",
            CallMethod::Probe => "probe",
        }
    }
}

/// 调用来源：手工调用 / 测试运行
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CallOrigin {
    #[default]
    Manual,
    Test,
}

impl CallOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallOrigin::Manual => "manual",
            CallOrigin::Test => "test",
        }
    }
}

/// 调用状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Ok,
    Error,
    Canceled,
}

impl CallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::Error => "error",
            CallStatus::Canceled => "canceled",
        }
    }
}

/// 单次调用的 JSON-RPC 原始帧（脱敏/截断前采集）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CallFrames {
    /// 线上 JSON-RPC id，保留 number / string 类型
    pub rpc_id: Option<serde_json::Value>,
    pub request: Option<serde_json::Value>,
    pub response: Option<serde_json::Value>,
    /// 任一帧因超限/异常未保存
    pub truncated: bool,
}

/// 历史记录（persist：call_records）
#[derive(Serialize, Clone, Debug)]
pub struct CallRecord {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub method: CallMethod,
    pub target: String,
    pub params: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<super::error::CallError>,
    pub status: CallStatus,
    pub duration_ms: u64,
    pub protocol_version: String,
    pub transport: TransportKind,
    pub truncated: bool,
    pub size_bytes: u64,
    #[serde(default)]
    pub origin: CallOrigin,
    /// 原始 JSON-RPC 帧；旧记录/不支持的方法为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<CallFrames>,
    pub created_at: DateTime<Utc>,
}

/// 历史摘要（列表 / 事件载荷）
#[derive(Serialize, Clone, Debug)]
pub struct CallRecordSummary {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub method: CallMethod,
    pub target: String,
    pub status: CallStatus,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub truncated: bool,
    #[serde(default)]
    pub origin: CallOrigin,
    pub created_at: DateTime<Utc>,
}

/// 服务器 OAuth 摘要（列表用；不读 keyring）
#[derive(Serialize, Clone, Debug)]
pub struct OAuthSummary {
    pub configured: bool,
    pub authorized: bool,
    pub refreshable: bool,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// 403 insufficient_scope 的 required_scope（v0.3.1）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<String>,
    /// 连接仍 Ready、但需要补充授权
    #[serde(default)]
    pub step_up_required: bool,
}

/// 服务器摘要（列表 / 状态事件）
#[derive(Serialize, Clone, Debug)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub transport: TransportKind,
    pub enabled: bool,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ServerOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthSummary>,
    pub last_connected_at: Option<DateTime<Utc>>,
    pub state: ConnectionState,
}

/// 健康探测结果
#[derive(Serialize, Clone, Debug)]
pub struct ProbeResult {
    pub ok: bool,
    pub rtt_ms: u64,
    pub detail: String,
}

/// 运行时设置快照（M4 接入持久化设置前的轻量结构）
#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    pub stderr_buffer_lines: usize,
    pub notification_buffer_lines: usize,
    /// 报文日志缓冲条数（v0.1.1）
    pub wire_buffer_lines: usize,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            request_timeout_secs: 30,
            connect_timeout_secs: 15,
            stderr_buffer_lines: 2000,
            notification_buffer_lines: 1000,
            wire_buffer_lines: 300,
        }
    }
}

/// 客户端支持的协议版本清单（新版本在前；与 02 文档 1.1 对齐）
pub const PREFERRED_PROTOCOL_VERSIONS: [rmcp::model::ProtocolVersion; 5] = [
    rmcp::model::ProtocolVersion::V_2026_07_28,
    rmcp::model::ProtocolVersion::V_2025_11_25,
    rmcp::model::ProtocolVersion::V_2025_06_18,
    rmcp::model::ProtocolVersion::V_2025_03_26,
    rmcp::model::ProtocolVersion::V_2024_11_05,
];

/// 旧世代回退版本（取最新 legacy，rmcp 官方示例口径）
pub const LEGACY_FALLBACK_VERSION: rmcp::model::ProtocolVersion =
    rmcp::model::ProtocolVersion::V_2025_11_25;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_http_config_without_oauth_deserializes() {
        let json = r#"{
            "url": "https://example.com/mcp",
            "headers": [],
            "timeout_secs": 10,
            "allow_insecure_tls": false
        }"#;
        let config: HttpConfig = serde_json::from_str(json).expect("旧配置反序列化失败");
        assert!(config.oauth.is_none());
    }

    #[test]
    fn oauth_config_roundtrip_skips_none() {
        let config = OAuthConfig {
            scopes: vec!["read".to_string()],
            client_id: Some("client".to_string()),
            redirect_uri: Some("http://127.0.0.1:33318/oauth/callback".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(&config).expect("序列化失败");
        assert_eq!(value["scopes"][0], "read");
        assert!(value.get("client_secret").is_none());
        let back: OAuthConfig = serde_json::from_value(value).expect("反序列化失败");
        assert_eq!(back, config);
    }
}
