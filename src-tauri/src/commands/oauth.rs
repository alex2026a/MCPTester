//! OAuth 登录命令（v0.3.0）

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::commands::AppState;
use crate::mcp::error::McpErrorInfo;
use crate::mcp::models::ConnectionState;
use crate::mcp::oauth::{
    OAuthAccessToken, OAuthDiagnostics, OAuthError, OAuthLoginStart, OAuthLogoutResult,
    OAuthRegistrationClearResult, OAuthStatus,
};

/// 开始浏览器 OAuth 登录
#[tauri::command]
pub async fn start_oauth_login(
    state: State<'_, AppState>,
    app: AppHandle,
    server_id: String,
) -> Result<OAuthLoginStart, McpErrorInfo> {
    let config = crate::commands::server::load_resolved_config(&state, &server_id)?;
    let http = config.http.as_ref().ok_or_else(|| {
        OAuthError::Config {
            message: "当前服务器没有 HTTP 配置".to_string(),
        }
        .to_info()
    })?;
    if http.oauth.is_none() {
        return Err(OAuthError::Config {
            message: "当前服务器未启用 OAuth".to_string(),
        }
        .to_info());
    }

    let mut challenge = None;
    if let Some(handle) = state.manager.handle(&server_id).await {
        let structured = handle.last_oauth_challenge().await.map(|c| c.raw);
        challenge = structured;
        if let ConnectionState::Error {
            error: crate::mcp::error::McpError::OAuthRequired { detail, .. },
        } = handle.state().await
        {
            challenge = detail.or(challenge);
        }
    }

    let mut login = state
        .oauth
        .begin_login(&server_id, http, challenge)
        .await
        .map_err(|e| e.to_info())?;
    match app.opener().open_url(login.auth_url.clone(), None::<&str>) {
        Ok(()) => login.browser_opened = true,
        Err(error) => login.open_error = Some(error.to_string()),
    }
    Ok(login)
}

/// 取消登录（按 flow_id）
#[tauri::command]
pub async fn cancel_oauth_login(
    state: State<'_, AppState>,
    flow_id: String,
) -> Result<(), McpErrorInfo> {
    state
        .oauth
        .cancel_login(&flow_id)
        .await
        .map_err(|e| e.to_info())
}

/// 查询登录状态
#[tauri::command]
pub async fn get_oauth_status(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthStatus, McpErrorInfo> {
    let config = state
        .db
        .with_read_blocking(|conn| crate::storage::server_repo::get(conn, &server_id))
        .ok()
        .flatten();
    let Some(http) = config
        .as_ref()
        .and_then(|config| config.http.as_ref())
        .filter(|http| http.oauth.is_some())
    else {
        return Ok(OAuthStatus {
            configured: false,
            authorized: false,
            refreshable: false,
            client_id: None,
            registration: None,
            client_metadata_url: None,
            token_endpoint_auth_method: None,
            scopes: Vec::new(),
            expires_at: None,
            issuer: None,
        });
    };
    let mut status = state
        .oauth
        .status_with_config(&server_id, http)
        .await
        .map_err(|e| e.to_info())?;
    if let Some(handle) = state.manager.handle(&server_id).await
        && matches!(
            handle.state().await,
            ConnectionState::Error {
                error: crate::mcp::error::McpError::OAuthRequired { .. }
            }
        )
    {
        status.authorized = false;
    }
    Ok(status)
}

/// 退出登录：清 token，保留客户端注册
#[tauri::command]
pub async fn logout_oauth(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthLogoutResult, McpErrorInfo> {
    let challenge = if let Some(handle) = state.manager.handle(&server_id).await {
        handle.last_oauth_challenge().await.map(|c| c.raw)
    } else {
        None
    };
    let config = crate::commands::server::load_resolved_config(&state, &server_id).ok();
    let http = config.as_ref().and_then(|config| config.http.as_ref());
    state.manager.disconnect(&server_id).await;
    state
        .oauth
        .logout(&server_id, http, challenge.as_deref())
        .await
        .map_err(|e| e.to_info())
}

/// OAuth 诊断（显式 discovery；不返回 secret/token）
#[tauri::command]
pub async fn get_oauth_diagnostics(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthDiagnostics, McpErrorInfo> {
    let config = state
        .db
        .with_read_blocking(|conn| crate::storage::server_repo::get(conn, &server_id))
        .map_err(|e| McpErrorInfo {
            kind: "oauth_config".to_string(),
            message: format!("读取服务器配置失败：{e}"),
            ..Default::default()
        })?
        .ok_or_else(|| McpErrorInfo {
            kind: "oauth_config".to_string(),
            message: "服务器不存在".to_string(),
            ..Default::default()
        })?;
    let http = config.http.as_ref().ok_or_else(|| McpErrorInfo {
        kind: "oauth_config".to_string(),
        message: "当前服务器没有 HTTP 配置".to_string(),
        ..Default::default()
    })?;
    if http.oauth.is_none() {
        return Err(McpErrorInfo {
            kind: "oauth_config".to_string(),
            message: "当前服务器未启用 OAuth".to_string(),
            ..Default::default()
        });
    }
    let challenge = if let Some(handle) = state.manager.handle(&server_id).await {
        handle.last_oauth_challenge().await.map(|c| c.raw)
    } else {
        None
    };
    state
        .oauth
        .diagnostics(&server_id, http, challenge.as_deref())
        .await
        .map_err(|e| e.to_info())
}

/// 强制刷新 OAuth token
#[tauri::command]
pub async fn refresh_oauth_token(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthStatus, McpErrorInfo> {
    let config = crate::commands::server::load_resolved_config(&state, &server_id)?;
    let http = config.http.as_ref().ok_or_else(|| McpErrorInfo {
        kind: "oauth_config".to_string(),
        message: "当前服务器没有 HTTP 配置".to_string(),
        ..Default::default()
    })?;
    if http.oauth.is_none() {
        return Err(McpErrorInfo {
            kind: "oauth_config".to_string(),
            message: "当前服务器未启用 OAuth".to_string(),
            ..Default::default()
        });
    }
    let challenge = if let Some(handle) = state.manager.handle(&server_id).await {
        handle.last_oauth_challenge().await.map(|c| c.raw)
    } else {
        None
    };
    state
        .oauth
        .force_refresh(&server_id, http, challenge.as_deref())
        .await
        .map_err(|e| e.to_info())
}

/// 查看当前 access token（显式用户操作）
#[tauri::command]
pub async fn get_oauth_access_token(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthAccessToken, McpErrorInfo> {
    let config = crate::commands::server::load_resolved_config(&state, &server_id)?;
    let http = config.http.as_ref().ok_or_else(|| McpErrorInfo {
        kind: "oauth_config".to_string(),
        message: "当前服务器没有 HTTP 配置".to_string(),
        ..Default::default()
    })?;
    if http.oauth.is_none() {
        return Err(McpErrorInfo {
            kind: "oauth_config".to_string(),
            message: "当前服务器未启用 OAuth".to_string(),
            ..Default::default()
        });
    }
    let challenge = if let Some(handle) = state.manager.handle(&server_id).await {
        handle.last_oauth_challenge().await.map(|c| c.raw)
    } else {
        None
    };
    state
        .oauth
        .access_token(&server_id, http, challenge.as_deref())
        .await
        .map_err(|e| e.to_info())
}

/// 清除客户端注册（DCR ↔ CIMD 切换用），保留服务器 OAuth 配置
#[tauri::command]
pub async fn clear_oauth_registration(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<OAuthRegistrationClearResult, McpErrorInfo> {
    state.manager.disconnect(&server_id).await;
    state
        .oauth
        .clear_registration(&server_id)
        .await
        .map_err(|e| e.to_info())
}
