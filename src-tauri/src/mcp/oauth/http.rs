//! 带 MCP 连接配置的 OAuth HTTP 客户端（v0.3.0 / v0.3.1）
//!
//! rmcp 默认客户端固定 30s、忽略项目的自签证书/代理设置。这里自实现
//! `OAuthHttpClient`，复用连接配置，并在出站前再次校验端点安全性。
//!
//! v0.3.1：把 `HttpConfig.headers` 中的非敏感自定义头带到 OAuth 请求。
//! 默认只发往与 MCP 资源同源的请求；跨源授权服务器需要显式
//! `forward_headers_to_auth_server`。带 header 的请求只跟随同源重定向。

use std::collections::HashSet;
use std::time::Duration;

use futures::StreamExt;
use oauth2::http::Response as OAuthResponse;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};
use url::{Origin, Url};

use crate::mcp::models::HttpConfig;

const MAX_BODY_BYTES: usize = 1024 * 1024;
const OAUTH_HTTP_TIMEOUT_SECS: u64 = 15;
const MAX_REDIRECTS: usize = 10;

/// 不随 OAuth 请求发送的头：OAuth 自己生成或属于 MCP 会话/传输层。
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "content-type",
    "content-length",
    "host",
    "connection",
    "cookie",
    "proxy-authorization",
    "mcp-session-id",
    "mcp-protocol-version",
    "last-event-id",
];

/// 双客户端 + 同源客户端：按请求要求选择是否跟随重定向
#[derive(Debug)]
pub struct McpOAuthHttpClient {
    follow_redirects: reqwest::Client,
    follow_same_origin: reqwest::Client,
    stop_redirects: reqwest::Client,
    headers: HeaderMap,
    resource_origin: Origin,
    forward_headers_to_auth_server: bool,
}

impl McpOAuthHttpClient {
    pub fn new(cfg: &HttpConfig) -> Result<Self, OAuthHttpClientError> {
        let timeout = Duration::from_secs(
            cfg.timeout_secs
                .unwrap_or(OAUTH_HTTP_TIMEOUT_SECS)
                .clamp(1, OAUTH_HTTP_TIMEOUT_SECS),
        );
        let resource = Url::parse(&cfg.url).map_err(|e| Box::new(e) as OAuthHttpClientError)?;
        let resource_origin = resource.origin();
        let headers = build_headers(cfg)?;
        let forward_headers_to_auth_server = cfg
            .oauth
            .as_ref()
            .is_some_and(|oauth| oauth.forward_headers_to_auth_server);

        let follow_redirects = build_client(
            timeout,
            cfg.allow_insecure_tls,
            redirect_policy(false, resource_origin.clone()),
        )?;
        let follow_same_origin = build_client(
            timeout,
            cfg.allow_insecure_tls,
            redirect_policy(true, resource_origin.clone()),
        )?;
        let stop_redirects = build_client(
            timeout,
            cfg.allow_insecure_tls,
            Policy::none(),
        )?;

        Ok(Self {
            follow_redirects,
            follow_same_origin,
            stop_redirects,
            headers,
            resource_origin,
            forward_headers_to_auth_server,
        })
    }

    fn validate_url(url: &Url) -> Result<(), OAuthHttpClientError> {
        let loopback = is_loopback(url);
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(Box::new(OAuthHttpError::InsecureUrl(url.to_string())));
        }
        Ok(())
    }

    fn should_apply_headers(&self, url: &Url) -> bool {
        self.forward_headers_to_auth_server || url.origin() == self.resource_origin
    }
}

fn build_client(
    timeout: Duration,
    allow_insecure_tls: bool,
    policy: Policy,
) -> Result<reqwest::Client, OAuthHttpClientError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .danger_accept_invalid_certs(allow_insecure_tls)
        .redirect(policy)
        .build()
        .map_err(|e| Box::new(e) as OAuthHttpClientError)
}

/// 重定向策略：
/// - 拒绝非 http(s)；
/// - 拒绝 HTTPS → HTTP；
/// - `same_origin_only` 时拒绝跨源；
/// - 最多 10 次。
fn redirect_policy(same_origin_only: bool, resource_origin: Origin) -> Policy {
    Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "too many OAuth redirects",
            ));
        }
        let next = attempt.url();
        if !matches!(next.scheme(), "http" | "https") {
            return attempt.error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "OAuth redirect to non-http(s) URL",
            ));
        }
        if let Some(previous) = attempt.previous().last()
            && previous.scheme() == "https"
            && next.scheme() == "http"
        {
            return attempt.error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "OAuth redirect from https to http is not allowed",
            ));
        }
        if same_origin_only && next.origin() != resource_origin {
            return attempt.stop();
        }
        attempt.follow()
    })
}

fn is_loopback(url: &Url) -> bool {
    url.host_str()
        .map(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "[::1]"
                || host == "::1"
        })
        .unwrap_or(false)
}

fn build_headers(cfg: &HttpConfig) -> Result<HeaderMap, OAuthHttpClientError> {
    let mut headers = HeaderMap::new();
    let mut seen = HashSet::new();
    for item in &cfg.headers {
        let key = item.key.trim();
        if key.is_empty() {
            return Err(Box::new(OAuthHttpError::InvalidHeader(
                "请求头名称不能为空".to_string(),
            )));
        }
        let lower = key.to_ascii_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        if !seen.insert(lower.clone()) {
            return Err(Box::new(OAuthHttpError::InvalidHeader(format!(
                "OAuth 请求头「{key}」重复"
            ))));
        }
        let Some(value) = item.value.as_deref().filter(|value| !value.is_empty()) else {
            // 掩码/未解析的 secret 头不发送空值
            continue;
        };
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| {
            Box::new(OAuthHttpError::InvalidHeader(format!(
                "请求头名称「{key}」非法：{e}"
            )))
        })?;
        let value = HeaderValue::from_str(value).map_err(|e| {
            Box::new(OAuthHttpError::InvalidHeader(format!(
                "请求头「{key}」的值非法：{e}"
            )))
        })?;
        headers.insert(name, value);
    }
    Ok(headers)
}

#[derive(Debug, thiserror::Error)]
enum OAuthHttpError {
    #[error("OAuth 端点必须是 https（loopback 除外）：{0}")]
    InsecureUrl(String),
    #[error("OAuth 请求头配置错误：{0}")]
    InvalidHeader(String),
    #[error("OAuth 响应体超过 {0} 字节")]
    BodyTooLarge(usize),
}

impl OAuthHttpClient for McpOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let OAuthHttpRequest {
                request,
                redirect_policy,
                ..
            } = request;
            let url = Url::parse(&request.uri().to_string())
                .map_err(|e| Box::new(e) as OAuthHttpClientError)?;
            Self::validate_url(&url)?;
            let apply_headers = self.should_apply_headers(&url);
            let client = match redirect_policy {
                OAuthHttpRedirectPolicy::Follow if apply_headers => &self.follow_same_origin,
                OAuthHttpRedirectPolicy::Follow => &self.follow_redirects,
                // 非穷尽枚举：未知策略保守地不跟随重定向
                _ => &self.stop_redirects,
            };
            let mut request = reqwest::Request::try_from(request)
                .map_err(|e| Box::new(e) as OAuthHttpClientError)?;
            if apply_headers {
                for (name, value) in &self.headers {
                    if !request.headers().contains_key(name) {
                        request.headers_mut().insert(name.clone(), value.clone());
                    }
                }
            }
            let response = client
                .execute(request)
                .await
                .map_err(|e| Box::new(e) as OAuthHttpClientError)?;
            let status = response.status();
            let headers = response.headers().clone();
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| Box::new(e) as OAuthHttpClientError)?;
                if chunk.len() > MAX_BODY_BYTES.saturating_sub(body.len()) {
                    return Err(Box::new(OAuthHttpError::BodyTooLarge(MAX_BODY_BYTES))
                        as OAuthHttpClientError);
                }
                body.extend_from_slice(&chunk);
            }
            let mut builder = OAuthResponse::builder().status(status);
            for (name, value) in headers.iter() {
                builder = builder.header(name, value);
            }
            builder
                .body(body)
                .map_err(|e| Box::new(e) as OAuthHttpClientError)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_config(headers: Vec<crate::mcp::models::KeyValue>) -> HttpConfig {
        HttpConfig {
            url: "https://mcp.example.com/mcp".to_string(),
            headers,
            timeout_secs: None,
            allow_insecure_tls: false,
            oauth: Some(crate::mcp::models::OAuthConfig::default()),
        }
    }

    fn key_value(key: &str, value: &str) -> crate::mcp::models::KeyValue {
        crate::mcp::models::KeyValue {
            key: key.to_string(),
            value: Some(value.to_string()),
            secret: false,
        }
    }

    #[test]
    fn headers_skip_reserved_and_reject_duplicates() {
        let cfg = http_config(vec![
            key_value("Authorization", "Bearer x"),
            key_value("X-Api-Key", "k1"),
            key_value("x-api-key", "k2"),
        ]);
        let error = McpOAuthHttpClient::new(&cfg).expect_err("重复 header 应失败");
        assert!(error.to_string().contains("重复"));
    }

    #[test]
    fn headers_are_built_without_reserved() {
        let cfg = http_config(vec![
            key_value("Authorization", "Bearer x"),
            key_value("X-Api-Key", "k1"),
            key_value("Mcp-Session-Id", "s1"),
        ]);
        let headers = build_headers(&cfg).expect("header 构建失败");
        assert_eq!(
            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
            Some("k1")
        );
        assert!(headers.get("authorization").is_none());
        assert!(headers.get("mcp-session-id").is_none());
    }

    #[test]
    fn same_origin_headers_default_off_for_cross_origin() {
        let cfg = http_config(vec![key_value("X-Api-Key", "k1")]);
        let client = McpOAuthHttpClient::new(&cfg).expect("client 创建失败");
        let same = Url::parse("https://mcp.example.com/.well-known/oauth-protected-resource")
            .expect("URL 解析失败");
        let cross = Url::parse("https://auth.example.com/token").expect("URL 解析失败");
        assert!(client.should_apply_headers(&same));
        assert!(!client.should_apply_headers(&cross));
    }

    #[test]
    fn forward_headers_to_auth_server_enables_cross_origin() {
        let mut cfg = http_config(vec![key_value("X-Api-Key", "k1")]);
        cfg.oauth.as_mut().expect("oauth").forward_headers_to_auth_server = true;
        let client = McpOAuthHttpClient::new(&cfg).expect("client 创建失败");
        let cross = Url::parse("https://auth.example.com/token").expect("URL 解析失败");
        assert!(client.should_apply_headers(&cross));
    }
}
