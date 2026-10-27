//! MCP 客户端错误分类（见设计文档 04 第 5 节）
//!
//! - `McpError`：连接级错误（spawn / 传输 / 握手 / 版本 / 认证 / 超时）
//! - `CallError`：调用级错误（RPC 错误 / 超时 / 传输关闭 / 校验 / 能力缺失 / MRTR）
//!
//! v0.1.1：新增 [`McpErrorInfo`]（结构化错误载荷：kind + 中文消息 + 原始细节），
//! 前端据此渲染"人话标题 + 修复建议 + 技术细节"（见 01 文档 4.3）。

use serde::{Deserialize, Serialize};

/// 连接级错误
#[derive(Serialize, Clone, Debug, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpError {
    /// 命令不存在、权限不足、cwd 无效等
    #[error("无法启动进程：{message}")]
    SpawnFailed { message: String },

    /// DNS / 连接被拒 / TLS
    #[error("连接失败：{message}")]
    TransportFailed { message: String },

    /// 协商、初始化失败
    #[error("握手失败：{message}")]
    HandshakeFailed { message: String },

    /// 协议版本不受支持
    #[error("协议版本不受支持（服务器支持：{supported:?}）")]
    UnsupportedProtocolVersion { supported: Vec<String> },

    /// HTTP 401/403
    #[error("认证失败（HTTP {status}）{suffix}", suffix = unauthorized_suffix(.detail))]
    Unauthorized {
        status: u16,
        /// 服务器 `WWW-Authenticate` 挑战原文（如有）
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    /// OAuth 服务器要求登录授权（v0.3.0）
    #[serde(rename = "oauth_required")]
    #[error("需要 OAuth 登录授权{suffix}", suffix = unauthorized_suffix(.detail))]
    OAuthRequired {
        /// 服务器 `WWW-Authenticate` 挑战原文（如有）
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// 从挑战解析的 Protected Resource Metadata URL
        #[serde(skip_serializing_if = "Option::is_none")]
        resource_metadata_url: Option<String>,
        /// 403 insufficient_scope 的 required_scope
        #[serde(skip_serializing_if = "Option::is_none")]
        required_scope: Option<String>,
    },

    /// 旧世代服务器请求客户端能力（现代世代不存在该场景）
    #[error("服务器请求了客户端能力 {capability}（v0.1 不支持）")]
    UnsupportedClientCapability { capability: String },

    /// 连接超时
    #[error("连接超时（{timeout_secs}s）")]
    ConnectTimeout { timeout_secs: u64 },

    /// 配置错误（如目录不存在）
    #[error("配置错误：{message}")]
    Config { message: String },

    /// 内部错误
    #[error("内部错误：{message}")]
    Internal { message: String },
}

/// `Unauthorized` 展示后缀（有挑战原文时附加）
fn unauthorized_suffix(detail: &Option<String>) -> String {
    detail
        .as_ref()
        .map(|text| format!("：{text}"))
        .unwrap_or_default()
}

/// 结构化连接错误（`connect_server` / `reconnect_server` 的失败载荷）
#[derive(Serialize, Clone, Debug, Default)]
pub struct McpErrorInfo {
    /// 错误类别（`McpError` serde tag，前端映射人话标题与修复建议）
    pub kind: String,
    /// 中文可读消息（UI 主文案）
    pub message: String,
    /// 原始技术细节（SDK 原文 / 挑战原文，详情弹窗展示）
    pub raw: Option<String>,
    /// 403 insufficient_scope 的 required_scope
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<String>,
    /// 从挑战解析的 Protected Resource Metadata URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_metadata_url: Option<String>,
}

impl McpError {
    /// 中文可读消息（UI 直接展示）
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// 错误类别（与 serde tag 一致；供前端错误映射使用）
    pub fn kind(&self) -> &'static str {
        match self {
            McpError::SpawnFailed { .. } => "spawn_failed",
            McpError::TransportFailed { .. } => "transport_failed",
            McpError::HandshakeFailed { .. } => "handshake_failed",
            McpError::UnsupportedProtocolVersion { .. } => "unsupported_protocol_version",
            McpError::Unauthorized { .. } => "unauthorized",
            McpError::OAuthRequired { .. } => "oauth_required",
            McpError::UnsupportedClientCapability { .. } => "unsupported_client_capability",
            McpError::ConnectTimeout { .. } => "connect_timeout",
            McpError::Config { .. } => "config",
            McpError::Internal { .. } => "internal",
        }
    }

    /// 原始技术细节（SDK / 服务器原文；无人话对应时为 None）
    pub fn raw(&self) -> Option<String> {
        match self {
            McpError::SpawnFailed { message }
            | McpError::TransportFailed { message }
            | McpError::HandshakeFailed { message }
            | McpError::Config { message }
            | McpError::Internal { message } => Some(message.clone()),
            McpError::Unauthorized { detail, .. } => detail.clone(),
            McpError::OAuthRequired {
                detail,
                resource_metadata_url,
                required_scope,
            } => {
                let mut parts = Vec::new();
                if let Some(detail) = detail {
                    parts.push(detail.clone());
                }
                if let Some(url) = resource_metadata_url {
                    parts.push(format!("resource_metadata={url}"));
                }
                if let Some(scope) = required_scope {
                    parts.push(format!("required_scope={scope}"));
                }
                (!parts.is_empty()).then(|| parts.join("；"))
            }
            _ => None,
        }
    }

    /// 结构化错误载荷
    pub fn to_info(&self) -> McpErrorInfo {
        let (required_scope, resource_metadata_url) = match self {
            McpError::OAuthRequired {
                required_scope,
                resource_metadata_url,
                ..
            } => (required_scope.clone(), resource_metadata_url.clone()),
            _ => (None, None),
        };
        McpErrorInfo {
            kind: self.kind().to_string(),
            message: self.message(),
            raw: self.raw(),
            required_scope,
            resource_metadata_url,
        }
    }
}

/// 调用级错误
#[derive(Serialize, Deserialize, Clone, Debug, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallError {
    /// 服务端返回 JSON-RPC 错误（code/message 原样透传）
    #[error("服务端返回错误：{code} {message}")]
    RpcError {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },

    /// 调用超时
    #[error("调用超时（{timeout_secs}s）")]
    Timeout { timeout_secs: u64 },

    /// 传输已关闭（进程退出 / HTTP 断流）
    #[error("传输已关闭：{reason}")]
    TransportClosed { reason: String },

    /// 参数校验失败（前端表单级 + Rust 类型级兜底）
    #[error("参数校验失败：{message}")]
    Validation { message: String },

    /// OAuth 认证失效（连接后 token 被撤销/刷新失败）
    #[serde(rename = "oauth_required")]
    #[error("需要 OAuth 登录授权{suffix}", suffix = unauthorized_suffix(.detail))]
    OAuthRequired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_metadata_url: Option<String>,
    },

    /// 非 step-up 的 403（服务器拒绝访问，不是“连接已关闭”）
    #[error("服务器拒绝访问（HTTP {status}）{suffix}", suffix = unauthorized_suffix(.detail))]
    Forbidden {
        status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    /// OAuth 临时认证失败（刷新/凭据存储），可重试
    #[error("OAuth 临时认证失败：{detail}")]
    AuthTransient { detail: String },

    /// MRTR：结果 `resultType:"input_required"`（v0.1 不支持交互式输入）
    #[error("需要交互式输入（MRTR input_required，v0.1 不支持）")]
    InputRequired,

    /// 服务器未声明该能力
    #[error("服务器未声明该能力：{capability}")]
    CapabilityMissing { capability: String },

    /// 内部错误
    #[error("内部错误：{message}")]
    Internal { message: String },
}

impl CallError {
    /// 将 rmcp 服务错误映射为调用错误（超时 / 传输关闭 / RPC 错误）
    pub fn from_service_error(
        err: &rmcp::service::ServiceError,
        timeout_secs: u64,
        oauth_enabled: bool,
        base_url: Option<&url::Url>,
    ) -> Self {
        use crate::mcp::connection::AuthFailure;
        use rmcp::service::ServiceError;
        if oauth_enabled
            && let ServiceError::TransportSend(error) = err
            && let Some(failure) = crate::mcp::connection::classify_oauth_service_error(
                error, base_url,
            )
        {
            return match failure {
                AuthFailure::Unauthorized(challenge)
                | AuthFailure::InsufficientScope(challenge) => CallError::OAuthRequired {
                    detail: Some(challenge.raw),
                    required_scope: challenge.required_scope,
                    resource_metadata_url: challenge.resource_metadata_url,
                },
                AuthFailure::Forbidden { status, detail } => CallError::Forbidden {
                    status,
                    detail: Some(detail),
                },
                AuthFailure::Transient(detail) => CallError::AuthTransient { detail },
            };
        }
        match err {
            ServiceError::Timeout { .. } => CallError::Timeout { timeout_secs },
            ServiceError::TransportClosed => CallError::TransportClosed {
                reason: "连接已关闭".to_string(),
            },
            ServiceError::TransportSend(e) => CallError::TransportClosed {
                reason: format!("发送失败：{e}"),
            },
            ServiceError::McpError(data) => CallError::RpcError {
                code: i64::from(data.code.0),
                message: data.message.to_string(),
                data: data.data.clone(),
            },
            ServiceError::Cancelled { reason } => CallError::TransportClosed {
                reason: reason.clone().unwrap_or_else(|| "请求已取消".to_string()),
            },
            other => CallError::Internal {
                message: other.to_string(),
            },
        }
    }
}
