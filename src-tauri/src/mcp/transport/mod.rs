//! 传输层（见设计文档 03 第 4 节）
//!
//! - stdio：`which_command` 解析 + 显式 JobObject/ProcessGroup 绑定 + stderr 管道读取
//! - streamable_http：rmcp 官方客户端（reqwest 后端）+ 自定义头 + 超时 + 可选不安全 TLS
//! - sse：自研遗留 HTTP+SSE（M2b；规范已弃用，兼容旧服务器）
//!
//! v0.1.1：Authorization 头归一化（`normalize_authorization`）——
//! 兼容"完整 `Bearer xxx`"与"仅 token"两种填写方式，并消除 HTTP / SSE 传输间行为差异。

pub mod http;
pub mod sse;
pub mod stdio;

/// Authorization 头归一化结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationValue {
    /// Bearer token（交给 rmcp `auth_header`，由客户端统一补 `Bearer ` 前缀）
    Bearer(String),
    /// 其他 scheme（如 `Basic xxx`）或需原样透传的完整头值
    Raw(String),
}

impl AuthorizationValue {
    /// 最终写入 HTTP 头的完整值
    pub fn header_value(&self) -> String {
        match self {
            AuthorizationValue::Bearer(token) => format!("Bearer {token}"),
            AuthorizationValue::Raw(raw) => raw.clone(),
        }
    }
}

/// 归一化 Authorization 头值（空值返回 `None`，调用方应跳过该头）：
///
/// | 输入 | 结果 |
/// |---|---|
/// | `Bearer xxx` / `bearer  xxx`（任意大小写、多余空白） | [`AuthorizationValue::Bearer`]("xxx") |
/// | 纯 token（无空白） | [`AuthorizationValue::Bearer`](token) |
/// | 其他 scheme（`Basic xxx` 等，首段为合法 scheme 标识） | [`AuthorizationValue::Raw`](原值) |
pub fn normalize_authorization(value: &str) -> Option<AuthorizationValue> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Bearer 前缀（大小写不敏感）：剥离后作为 token
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("bearer")
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        let token = trimmed["bearer".len()..].trim();
        if token.is_empty() {
            return None;
        }
        return Some(AuthorizationValue::Bearer(token.to_string()));
    }

    // 其他 scheme：`Scheme value` 形态（首段为合法 scheme 标识）原样透传
    if let Some((scheme, _)) = trimmed.split_once(char::is_whitespace)
        && is_scheme(scheme)
    {
        return Some(AuthorizationValue::Raw(trimmed.to_string()));
    }

    // 无 scheme 的纯 token：按 Bearer 处理
    Some(AuthorizationValue::Bearer(trimmed.to_string()))
}

/// 合法 scheme 标识（RFC 7235：token 字符）
fn is_scheme(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_full_bearer() {
        assert_eq!(
            normalize_authorization("Bearer mcp_abc123"),
            Some(AuthorizationValue::Bearer("mcp_abc123".to_string()))
        );
        assert_eq!(
            normalize_authorization("bearer  abc"),
            Some(AuthorizationValue::Bearer("abc".to_string()))
        );
        assert_eq!(
            normalize_authorization("  BEARER\tabc\t"),
            Some(AuthorizationValue::Bearer("abc".to_string()))
        );
    }

    #[test]
    fn normalize_plain_token() {
        assert_eq!(
            normalize_authorization("mcp_abc123"),
            Some(AuthorizationValue::Bearer("mcp_abc123".to_string()))
        );
    }

    #[test]
    fn normalize_other_scheme_raw() {
        assert_eq!(
            normalize_authorization("Basic dXNlcjpwYXNz"),
            Some(AuthorizationValue::Raw("Basic dXNlcjpwYXNz".to_string()))
        );
    }

    #[test]
    fn normalize_empty_and_bare_bearer() {
        assert_eq!(normalize_authorization(""), None);
        assert_eq!(normalize_authorization("   "), None);
        assert_eq!(normalize_authorization("Bearer"), None);
        assert_eq!(normalize_authorization("Bearer   "), None);
    }

    #[test]
    fn header_value_roundtrip() {
        assert_eq!(
            AuthorizationValue::Bearer("abc".to_string()).header_value(),
            "Bearer abc"
        );
        assert_eq!(
            AuthorizationValue::Raw("Basic xyz".to_string()).header_value(),
            "Basic xyz"
        );
    }
}

