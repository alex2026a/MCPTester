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
#[derive(Serialize, Clone, Debug)]
pub struct ToolInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
    pub annotations: Option<serde_json::Value>,
}

/// 资源信息
#[derive(Serialize, Clone, Debug)]
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
#[derive(Serialize, Clone, Debug)]
pub struct ResourceTemplateInfo {
    pub uri_template: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// 提示词参数
#[derive(Serialize, Clone, Debug)]
pub struct PromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

/// 提示词信息
#[derive(Serialize, Clone, Debug)]
pub struct PromptInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub arguments: Vec<PromptArgument>,
}

/// 能力快照（仅内存缓存）
#[derive(Serialize, Clone, Debug)]
pub struct CapabilitySnapshot {
    pub server_id: String,
    pub tools: Vec<ToolInfo>,
    pub resources: Vec<ResourceInfo>,
    pub resource_templates: Vec<ResourceTemplateInfo>,
    pub prompts: Vec<PromptInfo>,
    pub fetched_at: DateTime<Utc>,
    /// 是否因刷新失败而沿用旧快照
    pub stale: bool,
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
    pub created_at: DateTime<Utc>,
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
