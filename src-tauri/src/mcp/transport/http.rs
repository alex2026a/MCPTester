//! Streamable HTTP 传输：rmcp 官方客户端（reqwest 后端）
//!
//! - 自定义头（含 keyring 合并后的敏感头；`Authorization` 经归一化后走 auth_header 通道）
//! - 超时（服务器级覆盖 > 全局设置）；可选"允许不安全 TLS"（本地自签证书）
//!
//! v0.1.1：`Authorization: Bearer xxx`（完整头值）与仅 `xxx`（token）两种填法均可；
//! 其他 scheme（Basic 等）原样作为自定义头透传，避免 reqwest `bearer_auth` 盲目再拼前缀。
//!
//! v0.3.0：新增 `build_http_oauth`，只有 OAuth 模式使用 `AuthClient`；
//! 无 OAuth 的服务器继续走原 `reqwest::Client` 路径，避免 401 分类回归。

use std::collections::HashMap;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
use rmcp::transport::auth::{AuthClient, AuthorizationManager};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};

use super::{AuthorizationValue, normalize_authorization};
use crate::mcp::error::McpError;
use crate::mcp::models::HttpConfig;

struct PreparedHttp {
    client: reqwest::Client,
    config: StreamableHttpClientTransportConfig,
}

fn prepare(cfg: &HttpConfig, default_timeout_secs: u64) -> Result<PreparedHttp, McpError> {
    let mut auth_header: Option<String> = None;
    let mut custom_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();

    for kv in &cfg.headers {
        let Some(value) = kv.value.as_deref() else {
            continue;
        };
        let name = HeaderName::from_bytes(kv.key.as_bytes()).map_err(|e| McpError::Config {
            message: format!("请求头名称非法「{}」：{e}", kv.key),
        })?;

        if name == AUTHORIZATION {
            match normalize_authorization(value) {
                Some(AuthorizationValue::Bearer(token)) => {
                    // token 交给 rmcp，reqwest 统一补 "Bearer " 前缀
                    auth_header = Some(token);
                }
                Some(AuthorizationValue::Raw(raw)) => {
                    // 其他 scheme 原样透传（自定义头通道不做前缀处理）
                    let raw_value = HeaderValue::from_str(&raw).map_err(|e| McpError::Config {
                        message: format!("Authorization 头值非法：{e}"),
                    })?;
                    custom_headers.insert(name, raw_value);
                }
                None => {}
            }
            continue;
        }

        let value = HeaderValue::from_str(value).map_err(|e| McpError::Config {
            message: format!("请求头值非法「{}」：{e}", kv.key),
        })?;
        custom_headers.insert(name, value);
    }

    let timeout_secs = cfg.timeout_secs.unwrap_or(default_timeout_secs).max(1);
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(timeout_secs * 2));
    if cfg.allow_insecure_tls {
        // 仅服务器级显式开启（本地自签证书开发服务器）
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().map_err(|e| McpError::TransportFailed {
        message: format!("HTTP 客户端创建失败：{e}"),
    })?;

    let mut config = StreamableHttpClientTransportConfig::with_uri(cfg.url.clone());
    if let Some(auth) = auth_header.clone() {
        config = config.auth_header(auth);
    }
    if !custom_headers.is_empty() {
        config = config.custom_headers(custom_headers.clone());
    }
    // 旧世代会话过期时自动重新初始化（现代世代无会话，配置无副作用）
    config = config.reinit_on_expired_session(true);

    Ok(PreparedHttp { client, config })
}

/// 构建 Streamable HTTP 传输（无 OAuth）
pub fn build_http(
    cfg: &HttpConfig,
    default_timeout_secs: u64,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, McpError> {
    let prepared = prepare(cfg, default_timeout_secs)?;
    Ok(StreamableHttpClientTransport::with_client(
        prepared.client,
        prepared.config,
    ))
}

/// 构建 Streamable HTTP 传输（OAuth 模式）
pub fn build_http_oauth(
    cfg: &HttpConfig,
    default_timeout_secs: u64,
    manager: AuthorizationManager,
) -> Result<StreamableHttpClientTransport<AuthClient<reqwest::Client>>, McpError> {
    let prepared = prepare(cfg, default_timeout_secs)?;
    let auth_client = AuthClient::new(prepared.client, manager);
    Ok(StreamableHttpClientTransport::with_client(
        auth_client,
        prepared.config,
    ))
}
