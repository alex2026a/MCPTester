//! MCP OAuth 2.1 登录（v0.3.0）
//!
//! 只支持 Streamable HTTP；登录由用户显式触发，使用系统浏览器 + loopback
//! 回调。access token 只保留在进程内，持久化只写 refresh token。

pub mod callback;
pub mod error;
pub mod http;
pub mod identity;
pub mod service;
pub mod store;

pub use error::{OAuthError, map_auth_error, sanitize_detail};
pub use service::{
    AuthStateSink, OAuthAccessToken, OAuthDiagnostics, OAuthEvent, OAuthLoginStart,
    OAuthLogoutResult, OAuthRegistrationClearResult, OAuthService, OAuthStatus,
};
pub use store::{OAuthStoreMode, OAuthTokenCache};
