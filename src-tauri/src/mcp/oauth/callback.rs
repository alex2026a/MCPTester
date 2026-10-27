//! loopback OAuth 回调监听（v0.3.0）
//!
//! - 只绑定 loopback；
//! - `localhost` 同时尝试 v4/v6；
//! - 只接受精确 path，且 query 必须包含 `code` 与 `state`；
//! - 回调页只返回中性提示，state/issuer 校验由 rmcp session 完成。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use url::Url;

use super::error::OAuthError;

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_REQUEST_LINE_BYTES: usize = 2 * 1024;
const CALLBACK_TIMEOUT_SECS: u64 = 600;

/// 已绑定的回调监听器
pub struct CallbackListener {
    listeners: Vec<TcpListener>,
    /// 实际使用的 redirect_uri（不含 query）
    pub redirect_uri: String,
    /// 精确回调路径
    pub path: String,
    /// 非阻断提示（端口回退 / localhost v6 不可用）
    pub warning: Option<String>,
}

impl CallbackListener {
    pub fn port(&self) -> u16 {
        self.listeners
            .first()
            .and_then(|listener| listener.local_addr().ok())
            .map(|addr| addr.port())
            .unwrap_or(0)
    }
}

/// 解析并绑定 redirect_uri。
///
/// `allow_fallback` 仅用于动态注册：端口被占用时尝试后续 20 个端口；
/// 预注册/CIMD 必须保持精确 redirect，不自动回退。
pub async fn bind(
    redirect_uri: &str,
    allow_fallback: bool,
) -> Result<CallbackListener, OAuthError> {
    let url = Url::parse(redirect_uri).map_err(|e| OAuthError::InvalidRedirect {
        message: format!("URL 非法：{e}"),
    })?;
    if url.scheme() != "http" {
        return Err(OAuthError::InvalidRedirect {
            message: "回调地址必须使用 http（loopback）".to_string(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(OAuthError::InvalidRedirect {
            message: "回调地址不能包含 query 或 fragment".to_string(),
        });
    }
    let host = url.host_str().ok_or_else(|| OAuthError::InvalidRedirect {
        message: "回调地址缺少 host".to_string(),
    })?;
    let host_lower = host.to_ascii_lowercase();
    if !matches!(
        host_lower.as_str(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    ) {
        return Err(OAuthError::InvalidRedirect {
            message: "回调地址只允许 127.0.0.1 / localhost / [::1]".to_string(),
        });
    }
    let port = url.port().ok_or_else(|| OAuthError::InvalidRedirect {
        message: "回调地址必须显式指定端口".to_string(),
    })?;
    if port == 0 {
        return Err(OAuthError::InvalidRedirect {
            message: "回调端口不能为 0".to_string(),
        });
    }
    let path = if url.path().is_empty() {
        "/oauth/callback".to_string()
    } else {
        url.path().to_string()
    };

    let attempts = if allow_fallback { 20 } else { 1 };
    let mut last_error: Option<std::io::Error> = None;
    let mut bound = None;
    for offset in 0..attempts {
        let Some(candidate) = port.checked_add(offset) else {
            break;
        };
        if candidate == 0 {
            continue;
        }
        match try_bind(&host_lower, candidate, allow_fallback).await {
            Ok((listeners, host_for_uri, warning)) => {
                bound = Some((candidate, listeners, host_for_uri, warning));
                break;
            }
            Err(error) => {
                let retryable = error.kind() == std::io::ErrorKind::AddrInUse;
                tracing::warn!(
                    host = %host_lower,
                    port = candidate,
                    error = %error,
                    "OAuth 回调端口绑定失败"
                );
                last_error = Some(error);
                if !retryable || !allow_fallback {
                    break;
                }
            }
        }
    }

    let Some((actual_port, listeners, host_for_uri, mut warning)) = bound else {
        if let Some(error) = last_error {
            tracing::warn!(port, error = %error, "OAuth 回调端口全部不可用");
        }
        return Err(OAuthError::PortInUse { port });
    };

    if actual_port != port {
        let message = format!(
            "回调端口 {port} 被占用，已改用 {actual_port}；动态注册会按新端口重新登记"
        );
        warning = Some(match warning {
            Some(existing) => format!("{existing}；{message}"),
            None => message,
        });
    }

    Ok(CallbackListener {
        listeners,
        redirect_uri: format!(
            "{}://{}:{}{}",
            url.scheme(),
            host_for_uri,
            actual_port,
            path
        ),
        path,
        warning,
    })
}

async fn try_bind(
    host: &str,
    port: u16,
    allow_fallback: bool,
) -> Result<(Vec<TcpListener>, String, Option<String>), std::io::Error> {
    match host {
        "127.0.0.1" => {
            let listener = TcpListener::bind(("127.0.0.1", port)).await?;
            Ok((vec![listener], "127.0.0.1".to_string(), None))
        }
        "::1" | "[::1]" => {
            let listener = TcpListener::bind(("::1", port)).await?;
            Ok((vec![listener], "[::1]".to_string(), None))
        }
        _ => {
            // localhost：优先 v4，v6 可选（部分 Windows 环境 localhost 解析到 ::1）
            let v4 = TcpListener::bind(("127.0.0.1", port)).await?;
            let mut listeners = vec![v4];
            match TcpListener::bind(("::1", port)).await {
                Ok(v6) => {
                    listeners.push(v6);
                    Ok((listeners, "localhost".to_string(), None))
                }
                Err(error) => {
                    tracing::warn!(
                        port,
                        error = %error,
                        "localhost 的 IPv6 回调绑定失败"
                    );
                    if allow_fallback {
                        Ok((
                            listeners,
                            "127.0.0.1".to_string(),
                            Some(
                                "localhost 的 IPv6 回调不可用，已改用 127.0.0.1；\
                                 动态注册会按新地址重新登记"
                                    .to_string(),
                            ),
                        ))
                    } else {
                        Ok((
                            listeners,
                            "localhost".to_string(),
                            Some(
                                "localhost 的 IPv6 回调不可用；若浏览器回调失败，\
                                 请把回调地址 host 改为 127.0.0.1 并同步预注册/CIMD 文档"
                                    .to_string(),
                            ),
                        ))
                    }
                }
            }
        }
    }
}

/// 等待一次有效回调，返回完整 callback URL。
pub async fn wait_for_callback(
    listener: CallbackListener,
    mut cancel: watch::Receiver<bool>,
    timeout_secs: Option<u64>,
) -> Result<String, OAuthError> {
    let timeout_secs = timeout_secs.unwrap_or(CALLBACK_TIMEOUT_SECS);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let (tx, mut rx) = mpsc::unbounded_channel::<TcpStream>();
    let mut accept_tasks = Vec::new();
    for listener in listener.listeners {
        let tx = tx.clone();
        accept_tasks.push(tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if tx.send(stream).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    let result = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break Err(OAuthError::Timeout { timeout_secs });
        }
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break Err(OAuthError::Canceled);
                }
            }
            maybe_stream = tokio::time::timeout(remaining, rx.recv()) => {
                match maybe_stream {
                    Err(_) => break Err(OAuthError::Timeout { timeout_secs }),
                    Ok(None) => break Err(OAuthError::Internal {
                        message: "OAuth 回调监听已关闭".to_string(),
                    }),
                    Ok(Some(stream)) => {
                        match handle_stream(stream, &listener.path).await {
                            Ok(Some(query)) => {
                                let callback = format!("{}?{}", listener.redirect_uri, query);
                                break Ok(callback);
                            }
                            Ok(None) => continue,
                            Err(_) => continue,
                        }
                    }
                }
            }
        }
    };

    for task in accept_tasks {
        task.abort();
    }
    result
}

/// 返回 Some(query) 表示有效回调；None 表示无效请求但已响应，继续等待。
async fn handle_stream(
    mut stream: TcpStream,
    expected_path: &str,
) -> std::io::Result<Option<String>> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await;
        let read = match read {
            Ok(Ok(read)) => read,
            _ => return Ok(None),
        };
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_HEADER_BYTES {
            let _ = respond(&mut stream, 400, "请求过大").await;
            return Ok(None);
        }
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    let request_line = text.lines().next().unwrap_or_default();
    if request_line.len() > MAX_REQUEST_LINE_BYTES {
        let _ = respond(&mut stream, 400, "请求行过大").await;
        return Ok(None);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        let _ = respond(&mut stream, 405, "只支持 GET").await;
        return Ok(None);
    }
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        let _ = respond(&mut stream, 400, "回调地址非法").await;
        return Ok(None);
    };
    if url.path() != expected_path {
        let _ = respond(&mut stream, 404, "Not Found").await;
        return Ok(None);
    }
    let has_code = url
        .query_pairs()
        .any(|(key, value)| key == "code" && !value.is_empty());
    let has_state = url
        .query_pairs()
        .any(|(key, value)| key == "state" && !value.is_empty());
    if !has_code || !has_state {
        let _ = respond(&mut stream, 400, "缺少 code 或 state").await;
        return Ok(None);
    }
    let query = url.query().unwrap_or_default().to_string();
    let _ = respond(
        &mut stream,
        200,
        "授权结果已返回，请回到 MCP Tester 查看结果。",
    )
    .await;
    Ok(Some(query))
}

async fn respond(stream: &mut TcpStream, status: u16, message: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>MCP Tester</title></head>\
         <body style=\"font-family:sans-serif;padding:2rem\"><p>{message}</p></body></html>"
    );
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定失败");
        listener.local_addr().expect("地址失败").port()
    }

    async fn send_request(port: u16, path: &str) {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("连接失败");
        let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("发送失败");
        let mut buf = [0u8; 64];
        let _ = stream.read(&mut buf).await;
    }

    #[tokio::test]
    async fn waits_for_code_and_state() {
        let port = free_port().await;
        let listener = bind(&format!("http://127.0.0.1:{port}/oauth/callback"), false)
            .await
            .expect("绑定失败");
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(wait_for_callback(listener, cancel_rx, Some(10)));
        send_request(port, "/oauth/callback?foo=bar").await; // 无效，应继续等待
        send_request(port, "/oauth/callback?code=abc&state=xyz").await;
        let callback = task.await.expect("任务失败").expect("回调失败");
        assert!(callback.contains("code=abc"));
        assert!(callback.contains("state=xyz"));
    }

    #[tokio::test]
    async fn cancel_returns_canceled() {
        let port = free_port().await;
        let listener = bind(&format!("http://127.0.0.1:{port}/oauth/callback"), false)
            .await
            .expect("绑定失败");
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(wait_for_callback(listener, cancel_rx, Some(10)));
        cancel_tx.send(true).expect("发送取消失败");
        let result = task.await.expect("任务失败");
        assert!(matches!(result, Err(OAuthError::Canceled)));
    }

    #[tokio::test]
    async fn rejects_non_loopback_redirect() {
        let result = bind("http://example.com:33318/oauth/callback", false).await;
        assert!(matches!(result, Err(OAuthError::InvalidRedirect { .. })));
    }

    #[tokio::test]
    async fn falls_back_when_port_in_use() {
        let occupied = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("占用端口失败");
        let port = occupied.local_addr().expect("地址失败").port();
        let listener = bind(
            &format!("http://127.0.0.1:{port}/oauth/callback"),
            true,
        )
        .await
        .expect("回退绑定失败");
        assert_ne!(listener.port(), port);
        assert!(listener.warning.is_some());
    }

    #[tokio::test]
    async fn no_fallback_when_disabled() {
        let occupied = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("占用端口失败");
        let port = occupied.local_addr().expect("地址失败").port();
        let result = bind(
            &format!("http://127.0.0.1:{port}/oauth/callback"),
            false,
        )
        .await;
        assert!(matches!(
            result,
            Err(OAuthError::PortInUse { port: actual }) if actual == port
        ));
    }
}
