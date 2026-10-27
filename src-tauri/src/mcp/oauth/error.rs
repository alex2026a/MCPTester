//! OAuth 登录流程错误（v0.3.0）
//!
//! 登录流程错误与连接级 `McpError` 分开：命令层统一转 `McpErrorInfo`
//! 供前端展示。

use rmcp::transport::auth::AuthError;

use crate::mcp::error::McpErrorInfo;

/// OAuth 登录/存储错误
#[derive(Debug, Clone, thiserror::Error)]
pub enum OAuthError {
    #[error("OAuth 回调端口 {port} 被占用")]
    PortInUse { port: u16 },

    #[error("OAuth 回调地址非法：{message}")]
    InvalidRedirect { message: String },

    #[error("OAuth 元数据发现失败：{message}")]
    Metadata { message: String },

    #[error("OAuth 客户端注册失败：{message}")]
    Registration { message: String },

    #[error("OAuth token 交换失败：{message}")]
    TokenExchange { message: String },

    #[error("OAuth 登录超时（{timeout_secs}s）")]
    Timeout { timeout_secs: u64 },

    #[error("OAuth 登录已取消")]
    Canceled,

    #[error("OAuth 凭据存储失败：{message}")]
    Storage { message: String },

    #[error("OAuth 配置错误：{message}")]
    Config { message: String },

    #[error("OAuth 凭据初始化失败：{message}")]
    Materialize { message: String },

    #[error("当前没有可用的 OAuth token，请先登录")]
    NoToken,

    #[error("OAuth 内部错误：{message}")]
    Internal { message: String },
}

impl OAuthError {
    pub fn kind(&self) -> &'static str {
        match self {
            OAuthError::PortInUse { .. } => "oauth_port_in_use",
            OAuthError::InvalidRedirect { .. } => "oauth_invalid_redirect",
            OAuthError::Metadata { .. } => "oauth_metadata",
            OAuthError::Registration { .. } => "oauth_registration",
            OAuthError::TokenExchange { .. } => "oauth_token_exchange",
            OAuthError::Timeout { .. } => "oauth_timeout",
            OAuthError::Canceled => "oauth_canceled",
            OAuthError::Storage { .. } => "oauth_storage",
            OAuthError::Config { .. } => "oauth_config",
            OAuthError::Materialize { .. } => "oauth_internal",
            OAuthError::NoToken => "oauth_no_token",
            OAuthError::Internal { .. } => "oauth_internal",
        }
    }

    pub fn to_info(&self) -> McpErrorInfo {
        McpErrorInfo {
            kind: self.kind().to_string(),
            message: self.to_string(),
            raw: Some(sanitize_detail(&self.to_string())),
            ..Default::default()
        }
    }
}

/// 把 rmcp AuthError 映射为登录错误
pub fn map_auth_error(error: AuthError) -> OAuthError {
    match error {
        AuthError::RegistrationFailed(message) => OAuthError::Registration {
            message: sanitize_detail(&message),
        },
        AuthError::TokenExchangeFailed(message) => OAuthError::TokenExchange {
            message: sanitize_detail(&message),
        },
        AuthError::MetadataError(message) => OAuthError::Metadata {
            message: sanitize_detail(&message),
        },
        AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. }
        | AuthError::PkceUnsupported
        | AuthError::NoAuthorizationSupport => OAuthError::Metadata {
            message: sanitize_detail(&error.to_string()),
        },
        AuthError::AuthorizationRequired
        | AuthError::TokenRefreshRejected(_)
        | AuthError::TokenRefreshFailed(_) => OAuthError::TokenExchange {
            message: sanitize_detail(&error.to_string()),
        },
        other => OAuthError::Internal {
            message: sanitize_detail(&other.to_string()),
        },
    }
}

/// token / code / secret 脱敏 + 截断
pub fn sanitize_detail(text: &str) -> String {
    crate::logging::scrub_oauth_text(text)
}
