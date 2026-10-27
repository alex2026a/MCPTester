//! 服务器管理命令：CRUD + 连接生命周期 + 敏感值（keyring 版本化键名）双写
//!
//! 双写一致性见 04 文档 6.1：
//! - add：生成 nonce → 写 keyring → SQLite 事务（servers + credentials 索引），失败补偿；
//! - update：新 nonce 写 keyring → 事务切换索引 → 删除旧 ref（失败入 secret_cleanup）；
//! - delete：事务删 servers + 索引 → 尽力删除 keyring refs（失败入 secret_cleanup）。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::AppState;
use crate::commands::import_export::SECRET_PLACEHOLDER;
use crate::credentials::{self, SecretKind};
use crate::mcp::connection::ConnectionHandle;
use crate::mcp::error::McpErrorInfo;
use crate::mcp::models::{
    ConnectionState, Handshake, HttpConfig, KeyValue, OAuthSummary, ServerConfig, ServerSummary,
    StdioConfig, TransportKind,
};
use crate::mcp::wire::WireEntry;
use crate::storage::server_repo::{self, CredentialRef};

/// 敏感值输入（新增/更新时首次明文传入；keep/replace/clear 三态语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKindInput {
    Env,
    Header,
    /// serde 的 snake_case 会把 OAuth 转成 `o_auth`，而前端契约是 `oauth`，必须显式固定
    #[serde(rename = "oauth")]
    OAuth,
}

impl From<SecretKindInput> for SecretKind {
    fn from(value: SecretKindInput) -> Self {
        match value {
            SecretKindInput::Env => SecretKind::Env,
            SecretKindInput::Header => SecretKind::Header,
            SecretKindInput::OAuth => SecretKind::OAuth,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretAction {
    Keep,
    Replace,
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretInput {
    pub kind: SecretKindInput,
    pub key: String,
    pub action: SecretAction,
    pub value: Option<String>,
}

/// 服务器草稿（add/update 入参）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerDraft {
    pub name: String,
    pub transport: TransportKind,
    pub stdio: Option<StdioConfig>,
    pub http: Option<HttpConfig>,
    pub protocol_version_pin: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<SecretInput>,
}

fn default_true() -> bool {
    true
}

/// 列表（合并运行期连接状态）
#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>) -> Result<Vec<ServerSummary>, String> {
    let servers = state
        .db
        .with_read_blocking(server_repo::list)
        .map_err(|e| e.to_string())?;

    let oauth_summaries = state.oauth.summaries().await;
    let mut summaries = Vec::with_capacity(servers.len());
    for config in servers {
        let handle = state.manager.handle(&config.id).await;
        let state_value = match &handle {
            Some(handle) => handle.state().await,
            None => ConnectionState::Disconnected,
        };
        let challenge = match &handle {
            Some(handle) => handle.last_oauth_challenge().await,
            None => None,
        };
        let configured = config
            .http
            .as_ref()
            .and_then(|http| http.oauth.as_ref())
            .is_some();
        let oauth = configured.then(|| {
            let status = oauth_summaries.get(&config.id);
            OAuthSummary {
                configured: true,
                authorized: status.is_some_and(|status| status.authorized)
                    && !matches!(
                        state_value,
                        ConnectionState::Error {
                            error: crate::mcp::error::McpError::OAuthRequired { .. }
                        }
                    ),
                refreshable: status.is_some_and(|status| status.refreshable),
                scopes: status
                    .map(|status| status.scopes.clone())
                    .unwrap_or_default(),
                expires_at: status.and_then(|status| status.expires_at),
                required_scope: challenge
                    .as_ref()
                    .and_then(|challenge| challenge.required_scope.clone()),
                step_up_required: challenge
                    .as_ref()
                    .is_some_and(crate::mcp::connection::OAuthChallenge::is_insufficient_scope),
            }
        });
        summaries.push(ServerSummary {
            id: config.id,
            name: config.name,
            transport: config.transport,
            enabled: config.enabled,
            tags: config.tags,
            origin: config.origin,
            oauth,
            last_connected_at: config.last_connected_at,
            state: state_value,
        });
    }
    Ok(summaries)
}

/// 服务器详情（编辑用；敏感值为掩码/占位）
#[tauri::command]
pub async fn get_server(state: State<'_, AppState>, id: String) -> Result<ServerConfig, String> {
    let mut config = state
        .db
        .with_read_blocking(|conn| server_repo::get(conn, &id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "服务器不存在".to_string())?;
    if let Some(http) = config.http.as_mut()
        && let Some(oauth) = http.oauth.as_mut()
    {
        oauth.client_secret = state
            .oauth
            .has_user_client_secret(&id)
            .await
            .then(|| "****".to_string());
    }
    Ok(config)
}

/// 新增服务器
#[tauri::command]
pub async fn add_server(
    state: State<'_, AppState>,
    draft: ServerDraft,
) -> Result<ServerConfig, String> {
    add_server_inner(&state, draft).await
}

/// 新增服务器（内部实现：命令与导入流程共用）
pub(crate) async fn add_server_inner(
    state: &AppState,
    draft: ServerDraft,
) -> Result<ServerConfig, String> {
    validate_draft(&draft)?;
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();

    // 便携模式：凭据随数据目录明文保存（不写 keyring）
    if state.portable {
        let mut config = draft_to_config(&draft, id.clone(), now, now, false);
        apply_plaintext_secrets(&mut config, &draft.secrets, None);
        let config_for_db = config.clone();
        state
            .db
            .with_write_blocking(move |conn| server_repo::upsert(conn, &config_for_db))
            .map_err(|e| format!("保存服务器失败：{e}"))?;
        persist_oauth_secret(state, &id, &draft).await?;
        mask_oauth_secret(&mut config, state, &id).await;
        return Ok(config);
    }

    let config = draft_to_config(&draft, id.clone(), now, now, true);

    // 1. 敏感值写 keyring（生成版本化 ref）
    let mut written: Vec<(CredentialRef, SecretKind, String)> = Vec::new();
    for secret in &draft.secrets {
        if matches!(secret.kind, SecretKindInput::OAuth) {
            continue;
        }
        if secret.action != SecretAction::Replace && secret.action != SecretAction::Keep {
            continue;
        }
        let Some(value) = secret.value.as_deref().filter(|v| !v.is_empty()) else {
            continue;
        };
        let kind: SecretKind = secret.kind.clone().into();
        let nonce = credentials::new_nonce();
        let reference = credentials::secret_ref(&id, kind, &secret.key, &nonce);
        if let Err(e) = credentials::set_secret(&reference, value) {
            // 补偿：删除已写入条目
            for (item, _, _) in &written {
                let _ = credentials::delete_secret(&item.reference);
            }
            return Err(format!("敏感值写入失败：{e}"));
        }
        written.push((
            CredentialRef {
                server_id: id.clone(),
                kind: kind.path_segment().to_string(),
                name: secret.key.clone(),
                reference,
            },
            kind,
            value.to_string(),
        ));
    }

    // 2. SQLite 事务：config（敏感值仅掩码）+ credentials 索引
    let mut masked_config = config.clone();
    mask_config_secrets(&mut masked_config, &written);
    let result = state.db.with_write_blocking(|conn| {
        let tx = conn.transaction()?;
        server_repo::upsert(&tx, &masked_config)?;
        for (item, _, _) in &written {
            server_repo::upsert_credential(&tx, item)?;
        }
        tx.commit()?;
        Ok(())
    });
    if let Err(e) = result {
        for (item, _, _) in &written {
            let _ = credentials::delete_secret(&item.reference);
        }
        return Err(format!("保存服务器失败：{e}"));
    }

    persist_oauth_secret(state, &id, &draft).await?;
    mask_oauth_secret(&mut masked_config, state, &id).await;
    Ok(masked_config)
}

/// 保存 OAuth client_secret（真实值只进 oauth_credentials）
async fn persist_oauth_secret(
    state: &AppState,
    id: &str,
    draft: &ServerDraft,
) -> Result<(), String> {
    let Some(secret) = draft
        .secrets
        .iter()
        .find(|secret| matches!(secret.kind, SecretKindInput::OAuth))
    else {
        return Ok(());
    };
    let client_id = draft
        .http
        .as_ref()
        .and_then(|http| http.oauth.as_ref())
        .and_then(|oauth| oauth.client_id.clone());
    match secret.action {
        SecretAction::Keep => Ok(()),
        SecretAction::Clear => state
            .oauth
            .set_user_client_secret(id, client_id, None)
            .await
            .map_err(|e| e.to_string()),
        SecretAction::Replace => {
            let value = secret
                .value
                .as_deref()
                .filter(|value| !value.is_empty() && !looks_like_masked_secret(value))
                .ok_or_else(|| "client_secret 未提供新值".to_string())?;
            state
                .oauth
                .set_user_client_secret(id, client_id, Some(value.to_string()))
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// 回显 user client_secret 掩码（DCR / public client 不回显）
async fn mask_oauth_secret(config: &mut ServerConfig, state: &AppState, id: &str) {
    let Some(http) = config.http.as_mut() else {
        return;
    };
    let Some(oauth) = http.oauth.as_mut() else {
        return;
    };
    if state.oauth.has_user_client_secret(id).await {
        oauth.client_secret = Some("****".to_string());
    } else {
        oauth.client_secret = None;
    }
}

/// 更新服务器（敏感值三态：keep / replace / clear）
#[tauri::command]
pub async fn update_server(
    state: State<'_, AppState>,
    id: String,
    draft: ServerDraft,
) -> Result<ServerConfig, String> {
    update_server_inner(&state, id, draft).await
}

pub(crate) async fn update_server_inner(
    state: &AppState,
    id: String,
    draft: ServerDraft,
) -> Result<ServerConfig, String> {
    validate_draft(&draft)?;
    let existing = state
        .db
        .with_read_blocking(|conn| server_repo::get(conn, &id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "服务器不存在".to_string())?;

    let now = Utc::now();

    // 旧索引（用于 keep / clear / 删除旧 ref）
    let old_refs = state
        .db
        .with_read_blocking(|conn| server_repo::list_credentials(conn, &id))
        .map_err(|e| e.to_string())?;

    // 便携模式或配置文件来源：凭据明文入库，不写 keyring
    if state.portable || existing.origin.is_some() {
        let mut config = draft_to_config(&draft, id.clone(), existing.created_at, now, false);
        config.origin = existing.origin.clone();
        apply_plaintext_secrets(&mut config, &draft.secrets, Some(&existing));

        let config_for_db = config.clone();
        let id_for_tx = id.clone();
        state
            .db
            .with_write_blocking(move |conn| {
                let tx = conn.transaction()?;
                server_repo::upsert(&tx, &config_for_db)?;
                // 明文路径不再使用凭据索引；清理旧引用，避免 load 时覆盖
                server_repo::delete_credentials(&tx, &id_for_tx)?;
                tx.commit()?;
                Ok(())
            })
            .map_err(|e| format!("更新服务器失败：{e}"))?;

        // 尽力删除旧 keyring 条目（换机/切换模式后的残留）
        for item in &old_refs {
            if credentials::delete_secret(&item.reference).is_err() {
                let _ = state.db.with_write_blocking(|conn| {
                    server_repo::add_secret_cleanup(conn, &item.reference)
                });
            }
        }
        persist_oauth_secret(state, &id, &draft).await?;
        mask_oauth_secret(&mut config, state, &id).await;
        if let Some(http) = config.http.as_ref() {
            state
                .oauth
                .on_config_changed(&id, http)
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(config);
    }

    let mut config = draft_to_config(&draft, id.clone(), existing.created_at, now, true);
    config.origin = existing.origin.clone();

    let mut new_refs: Vec<(CredentialRef, String)> = Vec::new(); // (index, keyring ref)
    let mut to_delete_keys: Vec<(String, String)> = Vec::new(); // (kind, name) 旧值待删
    // v0.1.1：keep 语义保护——这些 (kind, name) 的旧 keyring 条目必须保留（修复此前被误删的问题）
    let mut keep_keys: Vec<(String, String)> = Vec::new();
    for secret in &draft.secrets {
        if matches!(secret.kind, SecretKindInput::OAuth) {
            continue;
        }
        let kind: SecretKind = secret.kind.clone().into();
        let kind_str = kind.path_segment().to_string();
        match secret.action {
            SecretAction::Keep => {
                keep_keys.push((kind_str, secret.key.clone()));
            }
            SecretAction::Clear => {
                to_delete_keys.push((kind_str, secret.key.clone()));
            }
            SecretAction::Replace => {
                let Some(value) = secret.value.as_deref().filter(|v| !v.is_empty()) else {
                    return Err(format!("敏感值「{}」未提供新值", secret.key));
                };
                // v0.1.1：掩码形态的值（含 "****"）不落库，按 keep 处理
                // （防止编辑表单回显掩码后被误写回凭据库，导致真实 token 丢失）
                if looks_like_masked_secret(value) {
                    tracing::warn!(
                        server_id = %id,
                        key = %secret.key,
                        "忽略掩码形态的敏感值替换（按 keep 处理）"
                    );
                    keep_keys.push((kind_str, secret.key.clone()));
                    continue;
                }
                let nonce = credentials::new_nonce();
                let reference = credentials::secret_ref(&id, kind, &secret.key, &nonce);
                credentials::set_secret(&reference, value)
                    .map_err(|e| format!("敏感值写入失败：{e}"))?;
                new_refs.push((
                    CredentialRef {
                        server_id: id.clone(),
                        kind: kind_str.clone(),
                        name: secret.key.clone(),
                        reference: reference.clone(),
                    },
                    reference,
                ));
                to_delete_keys.push((kind_str, secret.key.clone()));
            }
        }
    }

    // 敏感值掩码（结合旧索引：keep 的项显示掩码标记）
    mask_config_secrets_with_old(&mut config, &new_refs, &old_refs);

    let new_refs_clone = new_refs.clone();
    let delete_keys = to_delete_keys.clone();
    let config_for_tx = config.clone();
    let id_for_tx = id.clone();
    let result = state.db.with_write_blocking(move |conn| {
        let tx = conn.transaction()?;
        server_repo::upsert(&tx, &config_for_tx)?;
        for (item, _) in &new_refs_clone {
            server_repo::upsert_credential(&tx, item)?;
        }
        for (kind, name) in &delete_keys {
            server_repo::delete_credential(&tx, &id_for_tx, kind, name)?;
        }
        tx.commit()?;
        Ok(())
    });
    if let Err(e) = result {
        // 补偿：删除本次新写入的 keyring 条目
        for (_, reference) in &new_refs {
            let _ = credentials::delete_secret(reference);
        }
        return Err(format!("更新服务器失败：{e}"));
    }

    // 删除旧 keyring 条目（失败入 secret_cleanup）
    // v0.1.1：仅删除"非 keep"引用（keep 场景此前被误删，导致"留空=不修改"失效）
    for item in &old_refs {
        if keep_keys.contains(&(item.kind.clone(), item.name.clone())) {
            continue;
        }
        let failed = credentials::delete_secret(&item.reference).is_err();
        if failed {
            let _ = state
                .db
                .with_write_blocking(|conn| server_repo::add_secret_cleanup(conn, &item.reference));
        }
    }

    persist_oauth_secret(state, &id, &draft).await?;
    mask_oauth_secret(&mut config, state, &id).await;
    if let Some(http) = config.http.as_ref() {
        state
            .oauth
            .on_config_changed(&id, http)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(config)
}

/// 掩码形态判定（前端回显的掩码值：`Beare****w7aO` / `****（已配置）` 等）
///
/// 真实 token 几乎不可能包含连续的 `****`，出现即视为"未修改"语义。
pub(crate) fn looks_like_masked_secret(value: &str) -> bool {
    value.contains("****")
}

// ---------------------------------------------------------------------------
// 测试连接（v0.1.1；见 01 文档 4.4）
// ---------------------------------------------------------------------------

/// 测试步骤
#[derive(Debug, Clone, Serialize)]
pub struct TestStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub elapsed_ms: u64,
}

/// 连接测试报告（支持未保存草稿；不落库）
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionTestReport {
    pub ok: bool,
    pub steps: Vec<TestStep>,
    pub handshake: Option<Handshake>,
    pub tool_count: usize,
    pub resource_count: usize,
    pub template_count: usize,
    pub prompt_count: usize,
    pub error: Option<McpErrorInfo>,
    /// 本次测试连接产生的线上报文（临时句柄，不进入连接管理器）
    pub wire: Vec<WireEntry>,
}

/// 测试连接：传输建连 → 握手/协商 → 能力枚举 → 断开（全部在内存中完成）
#[tauri::command]
pub async fn test_server_connection(
    state: State<'_, AppState>,
    draft: ServerDraft,
    id: Option<String>,
) -> Result<ConnectionTestReport, McpErrorInfo> {
    let config = resolve_config_for_test(&state, id.as_deref(), &draft)?;
    let test_id = format!("__test__{}", uuid::Uuid::new_v4());
    let handle = ConnectionHandle::new(
        test_id.clone(),
        config.name.clone(),
        config.protocol_version_pin.clone(),
        config.transport.clone(),
        state.manager.settings().clone(),
    );

    let oauth_mode = match config.http.as_ref().and_then(|http| http.oauth.as_ref()) {
        Some(_) => {
            let saved = match id.as_deref() {
                Some(id) => state
                    .db
                    .with_read_blocking(|conn| server_repo::get(conn, id))
                    .ok()
                    .flatten(),
                None => None,
            };
            let same = saved.as_ref().is_some_and(|saved| {
                saved.http.as_ref().is_some_and(|old| {
                    config
                        .http
                        .as_ref()
                        .is_some_and(|new| old.url == new.url && old.oauth == new.oauth)
                })
            });
            Some(if same {
                crate::mcp::oauth::OAuthStoreMode::Persistent
            } else {
                crate::mcp::oauth::OAuthStoreMode::InMemory
            })
        }
        None => None,
    };
    let mut auth_manager = match (oauth_mode, config.http.as_ref()) {
        (Some(mode), Some(http)) => Some(
            state
                .oauth
                .manager_for(id.as_deref().unwrap_or(&test_id), http, mode)
                .await
                .map_err(|e| e.to_info())?,
        ),
        _ => None,
    };

    let mut steps: Vec<TestStep> = Vec::new();
    let report_failure =
        |steps: Vec<TestStep>, name: &str, detail: String, elapsed_ms: u64, error: McpErrorInfo| {
            let mut steps = steps;
            steps.push(TestStep {
                name: name.to_string(),
                ok: false,
                detail,
                elapsed_ms,
            });
            ConnectionTestReport {
                ok: false,
                steps,
                handshake: None,
                tool_count: 0,
                resource_count: 0,
                template_count: 0,
                prompt_count: 0,
                error: Some(error),
                wire: handle.wire().buffer(),
            }
        };

    // 1) 传输建连 + 握手/协商
    let started = std::time::Instant::now();
    let connect_result = match config.transport {
        TransportKind::Stdio => {
            let stdio = config
                .stdio
                .as_ref()
                .ok_or_else(|| config_error("缺少 stdio 配置"))?;
            handle.connect_stdio(stdio).await
        }
        TransportKind::StreamableHttp => {
            let http = config
                .http
                .as_ref()
                .ok_or_else(|| config_error("缺少 HTTP 配置"))?;
            handle.connect_http(http, auth_manager.take()).await
        }
        TransportKind::Sse => {
            let http = config
                .http
                .as_ref()
                .ok_or_else(|| config_error("缺少 SSE 配置"))?;
            handle.connect_sse(http).await
        }
    };
    if let Err(e) = connect_result {
        handle.disconnect().await;
        return Ok(report_failure(
            steps,
            "传输建连与握手",
            e.message(),
            started.elapsed().as_millis() as u64,
            e.to_info(),
        ));
    }
    let handshake = match handle.state().await {
        ConnectionState::Ready { handshake } => handshake,
        other => {
            handle.disconnect().await;
            let error = McpErrorInfo {
                kind: "internal".to_string(),
                message: format!("连接后状态异常：{other:?}"),
                raw: None,
                ..Default::default()
            };
            return Ok(report_failure(
                steps,
                "传输建连与握手",
                error.message.clone(),
                started.elapsed().as_millis() as u64,
                error,
            ));
        }
    };
    steps.push(TestStep {
        name: "传输建连与握手".to_string(),
        ok: true,
        detail: format!(
            "{} v{} · 协议 {} · {}",
            handshake.server_name,
            handshake.server_version,
            handshake.protocol_version,
            if handshake.lifecycle == crate::mcp::models::LifecycleEra::Modern {
                "现代世代"
            } else {
                "旧世代"
            }
        ),
        elapsed_ms: started.elapsed().as_millis() as u64,
    });

    // 2) 能力枚举
    let started = std::time::Instant::now();
    match handle.capabilities(true).await {
        Ok(snapshot) => {
            let mut detail = format!(
                "工具 {} · 资源 {} · 模板 {} · 提示词 {}",
                snapshot.tools.len(),
                snapshot.resources.len(),
                snapshot.resource_templates.len(),
                snapshot.prompts.len()
            );
            if !snapshot.warnings.is_empty() {
                detail.push_str(&format!("；{}", snapshot.warnings.join("；")));
            }
            steps.push(TestStep {
                name: "能力枚举".to_string(),
                ok: true,
                detail,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
            handle.disconnect().await;
            Ok(ConnectionTestReport {
                ok: true,
                steps,
                handshake: Some(handshake),
                tool_count: snapshot.tools.len(),
                resource_count: snapshot.resources.len(),
                template_count: snapshot.resource_templates.len(),
                prompt_count: snapshot.prompts.len(),
                error: None,
                wire: handle.wire().buffer(),
            })
        }
        Err(e) => {
            handle.disconnect().await;
            Ok(report_failure(
                steps,
                "能力枚举",
                e.message(),
                started.elapsed().as_millis() as u64,
                e.to_info(),
            ))
        }
    }
}

/// 由草稿构建测试配置（不落库）：Replace 明文直接使用；Keep/掩码按存储策略解析
fn resolve_config_for_test(
    state: &AppState,
    existing_id: Option<&str>,
    draft: &ServerDraft,
) -> Result<ServerConfig, McpErrorInfo> {
    validate_draft(draft).map_err(config_error)?;
    let now = Utc::now();
    let existing = match existing_id {
        Some(id) => state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, id))
            .map_err(|e| config_error(e.to_string()))?,
        None => None,
    };
    let plaintext = state.portable
        || existing
            .as_ref()
            .and_then(|item| item.origin.as_ref())
            .is_some();
    let mut config = draft_to_config(
        draft,
        existing_id.unwrap_or("test-preview").to_string(),
        now,
        now,
        !plaintext,
    );
    config.origin = existing.as_ref().and_then(|item| item.origin.clone());

    if plaintext {
        apply_plaintext_secrets(&mut config, &draft.secrets, existing.as_ref());
    } else {
        let old_refs = match existing_id {
            Some(id) => state
                .db
                .with_read_blocking(|conn| server_repo::list_credentials(conn, id))
                .map_err(|e| config_error(e.to_string()))?,
            None => Vec::new(),
        };

        for secret in &draft.secrets {
            if matches!(secret.kind, SecretKindInput::OAuth) {
                let value = match secret.action {
                    SecretAction::Clear => None,
                    SecretAction::Replace => {
                        let raw = secret.value.as_deref().unwrap_or_default();
                        if !raw.is_empty() && !looks_like_masked_secret(raw) {
                            Some(raw.to_string())
                        } else {
                            existing_id.and_then(|id| state.oauth.user_client_secret_blocking(id))
                        }
                    }
                    SecretAction::Keep => {
                        existing_id.and_then(|id| state.oauth.user_client_secret_blocking(id))
                    }
                };
                apply_secret_value(&mut config, "oauth", &secret.key, value);
                continue;
            }
            let kind: SecretKind = secret.kind.clone().into();
            let kind_str = kind.path_segment().to_string();
            let value = match secret.action {
                SecretAction::Clear => None,
                SecretAction::Replace => {
                    let raw = secret.value.as_deref().unwrap_or_default();
                    if !raw.is_empty() && !looks_like_masked_secret(raw) {
                        Some(raw.to_string())
                    } else {
                        // 掩码 / 空值：按 keep 从 keyring 解析
                        lookup_secret_value(&old_refs, &kind_str, &secret.key)?
                    }
                }
                SecretAction::Keep => lookup_secret_value(&old_refs, &kind_str, &secret.key)?,
            };
            apply_secret_value(&mut config, &kind_str, &secret.key, value);
        }
    }

    // v0.1.2：掩码遗留检测（与连接路径一致）
    check_pending_secrets(&config)?;
    check_masked_secrets(&config)?;
    Ok(config)
}

/// 从旧凭据索引读取 keep 敏感值
fn lookup_secret_value(
    refs: &[CredentialRef],
    kind: &str,
    name: &str,
) -> Result<Option<String>, McpErrorInfo> {
    match refs
        .iter()
        .find(|item| item.kind == kind && item.name == name)
    {
        Some(item) => {
            let value = credentials::get_secret(&item.reference).map_err(|e| {
                config_error(format!("敏感值「{name}」读取失败（{e}）：请重新填写"))
            })?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

/// 写入解析后的敏感值到配置槽位
fn apply_secret_value(config: &mut ServerConfig, kind: &str, name: &str, value: Option<String>) {
    let slot = match kind {
        "env" => config
            .stdio
            .as_mut()
            .and_then(|stdio| stdio.env.iter_mut().find(|kv| kv.key == name)),
        "header" => config
            .http
            .as_mut()
            .and_then(|http| http.headers.iter_mut().find(|kv| kv.key == name)),
        "oauth" => {
            if let Some(oauth) = config.http.as_mut().and_then(|http| http.oauth.as_mut()) {
                oauth.client_secret = value.clone();
            }
            None
        }
        _ => None,
    };
    if let Some(slot) = slot {
        slot.secret = true;
        slot.value = value;
    }
}

/// 删除服务器（默认保留历史；connection 一并释放）
#[tauri::command]
pub async fn delete_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.manager.remove(&id).await;
    let refs = state
        .db
        .with_read_blocking(|conn| server_repo::list_credentials(conn, &id))
        .map_err(|e| e.to_string())?;

    state
        .db
        .with_write_blocking(|conn| {
            let tx = conn.transaction()?;
            server_repo::delete_credentials(&tx, &id)?;
            server_repo::delete(&tx, &id)?;
            tx.commit()?;
            Ok(())
        })
        .map_err(|e| format!("删除服务器失败：{e}"))?;

    for item in refs {
        if credentials::delete_secret(&item.reference).is_err() {
            let _ = state
                .db
                .with_write_blocking(|conn| server_repo::add_secret_cleanup(conn, &item.reference));
        }
    }
    if let Err(e) = state.oauth.clear(&id).await {
        tracing::warn!(server_id = %id, error = %e, "清理 OAuth 凭据失败");
    }
    Ok(())
}

/// 连接（敏感值从 keyring 合并后连接；失败返回结构化错误）
#[tauri::command]
pub async fn connect_server(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionState, McpErrorInfo> {
    let config = load_resolved_config(&state, &id)?;
    let auth_manager = match config.http.as_ref() {
        Some(http) if http.oauth.is_some() => Some(
            state
                .oauth
                .manager_for(&id, http, crate::mcp::oauth::OAuthStoreMode::Persistent)
                .await
                .map_err(|e| e.to_info())?,
        ),
        _ => None,
    };
    let handle = state
        .manager
        .connect(&config, auth_manager)
        .await
        .map_err(|e| e.to_info())?;
    // 记录最后连接时间（仅成功时）
    let _ = state
        .db
        .with_write_blocking(|conn| server_repo::set_last_connected(conn, &id, Utc::now()));
    Ok(handle.state().await)
}

/// 断开
#[tauri::command]
pub async fn disconnect_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.manager.disconnect(&id).await;
    Ok(())
}

/// 重连
#[tauri::command]
pub async fn reconnect_server(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionState, McpErrorInfo> {
    state.manager.disconnect(&id).await;
    connect_server(state, id).await
}

/// 批量连接状态
#[tauri::command]
pub async fn get_connection_states(
    state: State<'_, AppState>,
) -> Result<Vec<ServerSummary>, String> {
    list_servers(state).await
}

/// 批量更新服务器名称/启用状态之外的完整草稿更新由 update_server 承担；
/// 此处提供轻量元信息更新（名称/启用）
#[tauri::command]
pub async fn update_server_meta(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    enabled: Option<bool>,
) -> Result<ServerConfig, String> {
    let mut config = state
        .db
        .with_read_blocking(|conn| server_repo::get(conn, &id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "服务器不存在".to_string())?;
    if let Some(name) = name {
        config.name = name;
    }
    if let Some(enabled) = enabled {
        config.enabled = enabled;
    }
    config.updated_at = Utc::now();
    let config_clone = config.clone();
    state
        .db
        .with_write_blocking(move |conn| server_repo::upsert(conn, &config_clone))
        .map_err(|e| format!("更新失败：{e}"))?;
    Ok(config)
}

/// 保存服务器自定义排序（要求提交当前全部服务器 ID）
#[tauri::command]
pub async fn reorder_servers(state: State<'_, AppState>, ids: Vec<String>) -> Result<(), String> {
    state
        .db
        .with_write(move |conn| server_repo::reorder(conn, &ids))
        .await
        .map_err(|e| format!("保存服务器顺序失败：{e}"))
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

pub(crate) fn validate_draft(draft: &ServerDraft) -> Result<(), String> {
    if draft.name.trim().is_empty() {
        return Err("服务器名称不能为空".to_string());
    }
    match draft.transport {
        TransportKind::Stdio => {
            let stdio = draft
                .stdio
                .as_ref()
                .ok_or_else(|| "缺少 stdio 配置".to_string())?;
            if stdio.command.trim().is_empty() {
                return Err("命令不能为空".to_string());
            }
            if draft
                .http
                .as_ref()
                .and_then(|http| http.oauth.as_ref())
                .is_some()
            {
                return Err("OAuth 仅支持 Streamable HTTP".to_string());
            }
        }
        TransportKind::StreamableHttp | TransportKind::Sse => {
            let http = draft
                .http
                .as_ref()
                .ok_or_else(|| "缺少 HTTP 配置".to_string())?;
            if !(http.url.starts_with("http://") || http.url.starts_with("https://")) {
                return Err("URL 必须以 http:// 或 https:// 开头".to_string());
            }
            if draft.transport == TransportKind::Sse && http.oauth.is_some() {
                return Err("OAuth 仅支持 Streamable HTTP，不支持遗留 SSE".to_string());
            }
            if let Some(oauth) = http.oauth.as_ref() {
                validate_oauth(http, oauth, &draft.secrets)?;
            }
        }
    }
    Ok(())
}

fn validate_oauth(
    http: &HttpConfig,
    oauth: &crate::mcp::models::OAuthConfig,
    secrets: &[SecretInput],
) -> Result<(), String> {
    if http
        .headers
        .iter()
        .any(|header| header.key.trim().eq_ignore_ascii_case("authorization"))
    {
        return Err("OAuth 模式下不能同时配置 Authorization 请求头".to_string());
    }
    let mut header_keys = std::collections::HashSet::new();
    for header in &http.headers {
        let key = header.key.trim().to_ascii_lowercase();
        if key.is_empty() {
            return Err("OAuth 模式下请求头名称不能为空".to_string());
        }
        if !header_keys.insert(key.clone()) {
            return Err(format!("OAuth 模式下请求头「{key}」重复"));
        }
    }
    ensure_oauth_url(&http.url, "MCP 端点")?;
    validate_authorize_params(oauth)?;
    if oauth.client_secret.is_some() && oauth.client_id.is_none() {
        return Err("client_secret 必须与 client_id 同时填写".to_string());
    }
    if let Some(url) = oauth.client_metadata_url.as_deref() {
        validate_client_metadata_url(url)?;
        if oauth.client_id.is_none()
            && (oauth.client_secret.is_some()
                || secrets.iter().any(|secret| {
                    matches!(secret.kind, SecretKindInput::OAuth)
                        && secret.action == SecretAction::Replace
                }))
        {
            return Err("CIMD 是公共客户端，不能同时配置 client_secret".to_string());
        }
    }
    if let Some(method) = oauth.token_endpoint_auth_method.as_deref()
        && !matches!(method, "client_secret_basic" | "client_secret_post" | "none")
    {
        return Err(
            "token_endpoint_auth_method 只能是 client_secret_basic/client_secret_post/none"
                .to_string(),
        );
    }
    for scope in &oauth.scopes {
        if scope.is_empty() || scope.chars().any(char::is_whitespace) {
            return Err(format!("scope「{scope}」不能为空或包含空白字符"));
        }
    }
    for endpoint in [
        oauth.authorization_endpoint.as_deref(),
        oauth.token_endpoint.as_deref(),
        oauth.registration_endpoint.as_deref(),
        oauth.revocation_endpoint.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        ensure_oauth_url(endpoint, "OAuth 端点")?;
    }
    if let Some(value) = oauth.revocation_endpoint.as_deref() {
        let url = url::Url::parse(value).map_err(|e| format!("撤销端点 URL 非法：{e}"))?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err("撤销端点不能包含 query 或 fragment".to_string());
        }
    }
    if let Some(redirect) = oauth.redirect_uri.as_deref() {
        validate_redirect_uri(redirect)?;
    }
    for secret in secrets {
        if matches!(secret.kind, SecretKindInput::OAuth) && secret.action == SecretAction::Replace {
            if oauth.client_id.is_none() {
                return Err("填写 client_secret 前必须先填写 client_id".to_string());
            }
            if secret
                .value
                .as_deref()
                .is_none_or(|value| value.is_empty() || looks_like_masked_secret(value))
            {
                return Err("client_secret 不能为空或掩码".to_string());
            }
        }
    }
    Ok(())
}

/// 授权请求额外参数：key 字符集、保留参数、重复 key、CR/LF。
fn validate_authorize_params(
    oauth: &crate::mcp::models::OAuthConfig,
) -> Result<(), String> {
    const RESERVED: &[&str] = &[
        "state",
        "code_challenge",
        "code_challenge_method",
        "client_id",
        "redirect_uri",
        "response_type",
        "scope",
        "resource",
    ];
    let mut seen = std::collections::HashSet::new();
    for param in &oauth.authorize_params {
        let key = param.key.trim();
        if key.is_empty() {
            return Err("授权请求参数 key 不能为空".to_string());
        }
        if !key.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
        }) {
            return Err(format!(
                "授权请求参数「{key}」只能包含字母、数字、_ - . :"
            ));
        }
        let lower = key.to_ascii_lowercase();
        if RESERVED.contains(&lower.as_str()) {
            return Err(format!("授权请求参数「{key}」是保留参数，不能覆盖"));
        }
        if !seen.insert(lower.clone()) {
            return Err(format!("授权请求参数「{key}」重复"));
        }
        if param.value.contains('\r') || param.value.contains('\n') {
            return Err(format!("授权请求参数「{key}」的值不能包含换行"));
        }
    }
    Ok(())
}

fn validate_client_metadata_url(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|e| format!("CIMD URL 非法：{e}"))?;
    if url.scheme() != "https" {
        return Err("CIMD URL 必须使用 https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("CIMD URL 不能包含用户名或密码".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("CIMD URL 不能包含 query 或 fragment".to_string());
    }
    if url.path().is_empty() || url.path() == "/" {
        return Err("CIMD URL 必须包含非根路径".to_string());
    }
    Ok(())
}

fn ensure_oauth_url(value: &str, label: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|e| format!("{label} URL 非法：{e}"))?;
    let loopback = url
        .host_str()
        .map(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "[::1]"
                || host == "::1"
        })
        .unwrap_or(false);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(format!("{label} 必须使用 https（loopback 除外）：{value}"));
    }
    Ok(())
}

fn validate_redirect_uri(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|e| format!("回调地址非法：{e}"))?;
    if url.scheme() != "http" {
        return Err("回调地址必须使用 http（loopback）".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("回调地址不能包含 query 或 fragment".to_string());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !matches!(
        host.as_str(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    ) {
        return Err("回调地址只允许 127.0.0.1 / localhost / [::1]".to_string());
    }
    if url.port().unwrap_or(0) == 0 {
        return Err("回调地址必须显式指定非 0 端口".to_string());
    }
    Ok(())
}

pub(crate) fn draft_to_config(
    draft: &ServerDraft,
    id: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    mask_secrets: bool,
) -> ServerConfig {
    let mut stdio = draft.stdio.clone();
    let mut http = draft.http.clone();
    // OAuth client_secret 的单一事实源是 oauth_credentials，config_json 永不落值
    if let Some(oauth) = http.as_mut().and_then(|http| http.oauth.as_mut()) {
        oauth.client_secret = None;
    }
    if mask_secrets {
        if let Some(config) = stdio.as_mut() {
            apply_masks(&mut config.env, &draft.secrets, SecretKindInput::Env);
        }
        if let Some(config) = http.as_mut() {
            apply_masks(&mut config.headers, &draft.secrets, SecretKindInput::Header);
        }
    }
    ServerConfig {
        id,
        name: draft.name.clone(),
        transport: draft.transport.clone(),
        stdio,
        http,
        protocol_version_pin: draft.protocol_version_pin.clone(),
        enabled: draft.enabled,
        tags: draft.tags.clone(),
        origin: None,
        created_at,
        updated_at,
        last_connected_at: None,
    }
}

/// 明文凭据模式：保留/写入真实值，仅维护 `secret` 标记（导出时仍脱敏）。
///
/// 用于便携模式与配置文件来源的服务器；`existing` 用于 keep/掩码值回填。
pub(crate) fn apply_plaintext_secrets(
    config: &mut ServerConfig,
    secrets: &[SecretInput],
    existing: Option<&ServerConfig>,
) {
    for secret in secrets {
        let (items, kind) = match secret.kind {
            SecretKindInput::Env => (config.stdio.as_mut().map(|stdio| &mut stdio.env), "env"),
            SecretKindInput::Header => {
                (config.http.as_mut().map(|http| &mut http.headers), "header")
            }
            // OAuth client_secret 由 OAuthService 存 oauth_credentials，不写 config
            SecretKindInput::OAuth => continue,
        };
        let Some(items) = items else { continue };
        let Some(item) = items.iter_mut().find(|item| item.key == secret.key) else {
            continue;
        };

        match secret.action {
            SecretAction::Clear => {
                item.value = None;
                item.secret = true;
            }
            SecretAction::Replace => {
                let value = secret.value.as_deref().filter(|value| {
                    !value.is_empty()
                        && !looks_like_masked_secret(value)
                        && *value != SECRET_PLACEHOLDER
                });
                if let Some(value) = value {
                    item.value = Some(value.to_string());
                } else if let Some(old) = find_plaintext_value(existing, kind, &secret.key) {
                    item.value = old.value.clone();
                }
                item.secret = true;
            }
            SecretAction::Keep => {
                let needs_old = item.value.as_deref().is_none_or(|value| {
                    value.is_empty()
                        || looks_like_masked_secret(value)
                        || value == SECRET_PLACEHOLDER
                });
                if needs_old && let Some(old) = find_plaintext_value(existing, kind, &secret.key) {
                    item.value = old.value.clone();
                }
                item.secret = true;
            }
        }
    }
}

fn find_plaintext_value<'a>(
    config: Option<&'a ServerConfig>,
    kind: &str,
    key: &str,
) -> Option<&'a KeyValue> {
    match kind {
        "env" => config
            .and_then(|config| config.stdio.as_ref())
            .and_then(|stdio| stdio.env.iter().find(|item| item.key == key)),
        "header" => config
            .and_then(|config| config.http.as_ref())
            .and_then(|http| http.headers.iter().find(|item| item.key == key)),
        _ => None,
    }
}

/// 将草稿中的敏感值替换为掩码（不落明文）
fn apply_masks(items: &mut [KeyValue], secrets: &[SecretInput], kind: SecretKindInput) {
    for item in items.iter_mut() {
        let matched = secrets.iter().find(|s| {
            matches!(
                (&s.kind, &kind),
                (SecretKindInput::Env, SecretKindInput::Env)
                    | (SecretKindInput::Header, SecretKindInput::Header)
            ) && s.key == item.key
        });
        if let Some(secret) = matched {
            item.secret = true;
            item.value = match secret.action {
                SecretAction::Clear => None,
                SecretAction::Replace => secret
                    .value
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .map(credentials::mask),
                // keep：不回显明文（前端以"已配置"占位展示）
                SecretAction::Keep => None,
            };
        }
    }
}

fn mask_config_secrets(config: &mut ServerConfig, written: &[(CredentialRef, SecretKind, String)]) {
    if let Some(stdio) = config.stdio.as_mut() {
        for item in stdio.env.iter_mut() {
            if let Some((_, _, value)) = written
                .iter()
                .find(|(r, k, _)| r.name == item.key && *k == SecretKind::Env)
            {
                item.secret = true;
                item.value = Some(credentials::mask(value));
            }
        }
    }
    if let Some(http) = config.http.as_mut() {
        for item in http.headers.iter_mut() {
            if let Some((_, _, value)) = written
                .iter()
                .find(|(r, k, _)| r.name == item.key && *k == SecretKind::Header)
            {
                item.secret = true;
                item.value = Some(credentials::mask(value));
            }
        }
    }
}

fn mask_config_secrets_with_old(
    config: &mut ServerConfig,
    new_refs: &[(CredentialRef, String)],
    old_refs: &[CredentialRef],
) {
    if let Some(stdio) = config.stdio.as_mut() {
        for item in stdio.env.iter_mut() {
            apply_old_mask(item, "env", new_refs, old_refs);
        }
    }
    if let Some(http) = config.http.as_mut() {
        for item in http.headers.iter_mut() {
            apply_old_mask(item, "header", new_refs, old_refs);
        }
    }
}

fn apply_old_mask(
    item: &mut KeyValue,
    kind: &str,
    new_refs: &[(CredentialRef, String)],
    old_refs: &[CredentialRef],
) {
    let has_new = new_refs
        .iter()
        .any(|(r, _)| r.kind == kind && r.name == item.key);
    let has_old = old_refs
        .iter()
        .any(|r| r.kind == kind && r.name == item.key);
    if has_new || has_old {
        item.secret = true;
        if !has_new {
            // keep 场景：值不落库，仅标记为已配置
            item.value = Some("****（已配置）".to_string());
        }
    }
}

/// 读取配置并把 keyring 中的敏感值合并回运行期配置
/// （v0.1.2：错误结构化；合并后检测"掩码遗留"并给出定向提示）
///
/// 便携模式与配置文件来源的服务器凭据以明文存于 config_json，
/// 不读取 keyring（避免旧引用覆盖明文）。
pub(crate) fn load_resolved_config(
    state: &AppState,
    id: &str,
) -> Result<ServerConfig, McpErrorInfo> {
    let mut config = state
        .db
        .with_read_blocking(|conn| server_repo::get(conn, id))
        .map_err(|e| config_error(e.to_string()))?
        .ok_or_else(|| config_error("服务器不存在".to_string()))?;

    let plaintext = state.portable || config.origin.is_some();
    if !plaintext {
        let refs = state
            .db
            .with_read_blocking(|conn| server_repo::list_credentials(conn, id))
            .map_err(|e| config_error(e.to_string()))?;

        for item in refs {
            let value = match credentials::get_secret(&item.reference) {
                Ok(value) => value,
                Err(e) => {
                    return Err(config_error(format!(
                        "敏感值「{}」读取失败（{}）：请重新配置",
                        item.name, e
                    )));
                }
            };
            let target = match item.kind.as_str() {
                "env" => config
                    .stdio
                    .as_mut()
                    .map(|s| s.env.iter_mut().find(|kv| kv.key == item.name)),
                "header" => config
                    .http
                    .as_mut()
                    .map(|h| h.headers.iter_mut().find(|kv| kv.key == item.name)),
                _ => None,
            };
            if let Some(Some(slot)) = target {
                slot.value = Some(value);
                slot.secret = true;
            }
        }
    }

    // OAuth：只注入 user 预注册 secret；DCR secret 由 OAuthService 自行读取
    if let Some(oauth) = config.http.as_mut().and_then(|http| http.oauth.as_mut())
        && let Some(secret) = state.oauth.user_client_secret_blocking(id)
    {
        oauth.client_secret = Some(secret);
    }

    // 待配置占位符：给出可操作错误，而不是带着空值去连服务器
    check_pending_secrets(&config)?;
    // v0.1.2：掩码遗留检测（旧版本编辑保存曾把掩码写回凭据库）
    check_masked_secrets(&config)?;
    Ok(config)
}

/// 配置类结构化错误（kind=config）
pub(crate) fn config_error(message: impl Into<String>) -> McpErrorInfo {
    McpErrorInfo {
        kind: "config".to_string(),
        message: message.into(),
        raw: None,
        ..Default::default()
    }
}

/// 掩码遗留检测：secret 值为掩码形态（含 `****`）时返回定向错误（v0.1.2）
///
/// 直接原因：旧版本编辑保存会把表单回显的
/// 掩码写回凭据库；v0.1.1 起阻止新损坏，但已存坏值需要用户重新填写——
/// 这里在连接前明确提示，而不是等服务器 401。
fn check_masked_secrets(config: &ServerConfig) -> Result<(), McpErrorInfo> {
    if let Some(stdio) = &config.stdio {
        check_masked_items(&stdio.env, "环境变量")?;
    }
    if let Some(http) = &config.http {
        check_masked_items(&http.headers, "请求头")?;
    }
    Ok(())
}

fn check_masked_items(items: &[KeyValue], scope: &str) -> Result<(), McpErrorInfo> {
    for kv in items {
        if kv.secret
            && let Some(value) = kv.value.as_deref()
            && looks_like_masked_secret(value)
        {
            return Err(McpErrorInfo {
                kind: "credential_mask".to_string(),
                message: format!(
                    "{scope}「{}」当前为掩码（由旧版本编辑保存遗留），无法用于连接。\
                     请编辑服务器：清空该输入框并重新填写真实值后重试",
                    kv.key
                ),
                raw: None,
                ..Default::default()
            });
        }
    }
    Ok(())
}

/// 待配置检测：secret 项为空或仍是导出占位符时给出明确提示
fn check_pending_secrets(config: &ServerConfig) -> Result<(), McpErrorInfo> {
    if let Some(stdio) = &config.stdio {
        check_pending_items(&stdio.env, "环境变量")?;
    }
    if let Some(http) = &config.http {
        check_pending_items(&http.headers, "请求头")?;
    }
    Ok(())
}

fn check_pending_items(items: &[KeyValue], scope: &str) -> Result<(), McpErrorInfo> {
    for kv in items {
        if !kv.secret {
            continue;
        }
        let pending = match kv.value.as_deref() {
            None => true,
            Some(value) => value.is_empty() || value == SECRET_PLACEHOLDER,
        };
        if pending {
            return Err(McpErrorInfo {
                kind: "credential_pending".to_string(),
                message: format!(
                    "{scope}「{}」尚未配置。请编辑服务器填写真实值后重试\
                     （导出配置中的 ${{MCPTESTER_SECRET}} 占位符不会自动还原）",
                    kv.key
                ),
                raw: None,
                ..Default::default()
            });
        }
    }
    Ok(())
}

/// 启动自检：重试待清理的 keyring 条目 + 清理孤儿索引（server_id 不在 servers 表）
pub fn startup_secret_check(state: &AppState) {
    // 1. secret_cleanup 队列重试
    let cleanups = state
        .db
        .with_read_blocking(server_repo::list_secret_cleanup)
        .unwrap_or_default();
    for reference in cleanups {
        if credentials::delete_secret(&reference).is_ok() {
            let _ = state
                .db
                .with_write_blocking(|conn| server_repo::remove_secret_cleanup(conn, &reference));
        }
    }

    // 2. 孤儿索引清理（含已删除服务器的残留 ref）
    let all = state
        .db
        .with_read_blocking(server_repo::list_all_credentials)
        .unwrap_or_default();
    for item in all {
        let exists = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, &item.server_id))
            .ok()
            .flatten()
            .is_some();
        if !exists {
            tracing::warn!(server_id = %item.server_id, "清理孤儿凭据索引");
            let _ = credentials::delete_secret(&item.reference);
            let _ = state.db.with_write_blocking(|conn| {
                server_repo::delete_credential(conn, &item.server_id, &item.kind, &item.name)
            });
        }
    }

    // 3. OAuth 孤儿清理（v0.3.0）：服务器已删除 / refresh 分块缺失
    let oauth_rows = state
        .db
        .with_read_blocking(crate::storage::oauth_repo::list_all)
        .unwrap_or_default();
    for row in oauth_rows {
        let exists = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, &row.server_id))
            .ok()
            .flatten()
            .is_some();
        if !exists {
            tracing::warn!(server_id = %row.server_id, "清理孤儿 OAuth 凭据");
            let base = crate::mcp::oauth::store::refresh_ref(&row.server_id);
            let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
            if let Some(reference) = row.client_secret_ref.as_deref() {
                let _ = credentials::delete_secret(reference);
            }
            let server_id = row.server_id.clone();
            let _ = state
                .db
                .with_write_blocking(|conn| crate::storage::oauth_repo::delete(conn, &server_id));
        } else if row.has_refresh_token && !state.portable {
            let base = crate::mcp::oauth::store::refresh_ref(&row.server_id);
            let readable = row.refresh_ref.as_deref().is_some_and(|reference| {
                credentials::get_secret_chunked(reference, row.refresh_chunks.max(0) as u32).is_ok()
            });
            if !readable {
                tracing::warn!(server_id = %row.server_id, "清理缺失的 OAuth refresh token");
                let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
                let server_id = row.server_id.clone();
                let _ = state.db.with_write_blocking(|conn| {
                    crate::storage::oauth_repo::clear_token(conn, &server_id)
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ConnectionManager;
    use crate::mcp::models::RuntimeSettings;
    use crate::storage::Database;
    use std::sync::Arc;

    fn oauth_draft() -> ServerDraft {
        ServerDraft {
            name: "oauth".to_string(),
            transport: TransportKind::StreamableHttp,
            stdio: None,
            http: Some(HttpConfig {
                url: "https://example.com/mcp".to_string(),
                headers: Vec::new(),
                timeout_secs: None,
                allow_insecure_tls: false,
                oauth: Some(crate::mcp::models::OAuthConfig::default()),
            }),
            protocol_version_pin: None,
            enabled: true,
            tags: Vec::new(),
            secrets: Vec::new(),
        }
    }

    #[test]
    fn oauth_validation_rejects_sse_and_conflicts() {
        let mut draft = oauth_draft();
        draft.transport = TransportKind::Sse;
        assert!(validate_draft(&draft).is_err(), "SSE 不允许 OAuth");

        let mut draft = oauth_draft();
        draft.http.as_mut().unwrap().headers.push(KeyValue {
            key: "Authorization".to_string(),
            value: Some("Bearer x".to_string()),
            secret: true,
        });
        assert!(
            validate_draft(&draft).is_err(),
            "OAuth 与 Authorization 冲突"
        );

        let mut draft = oauth_draft();
        draft.http.as_mut().unwrap().url = "http://example.com/mcp".to_string();
        assert!(validate_draft(&draft).is_err(), "远端 http 资源不允许");

        let mut draft = oauth_draft();
        draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap()
            .redirect_uri = Some("http://127.0.0.1:33318/callback?x=1".to_string());
        assert!(validate_draft(&draft).is_err(), "redirect 不允许 query");

        let mut draft = oauth_draft();
        draft.secrets.push(SecretInput {
            kind: SecretKindInput::OAuth,
            key: "client_secret".to_string(),
            action: SecretAction::Replace,
            value: Some("secret".to_string()),
        });
        assert!(validate_draft(&draft).is_err(), "secret 需要 client_id");
    }

    #[test]
    fn oauth_validation_rejects_duplicate_headers_and_reserved_params() {
        let mut draft = oauth_draft();
        draft.http.as_mut().unwrap().headers = vec![
            KeyValue {
                key: "X-Api-Key".to_string(),
                value: Some("a".to_string()),
                secret: false,
            },
            KeyValue {
                key: "x-api-key".to_string(),
                value: Some("b".to_string()),
                secret: false,
            },
        ];
        assert!(validate_draft(&draft).is_err(), "重复 header 应被拒绝");

        let mut draft = oauth_draft();
        draft.http.as_mut().unwrap().headers = vec![KeyValue {
            key: "Authorization".to_string(),
            value: None,
            secret: true,
        }];
        assert!(
            validate_draft(&draft).is_err(),
            "secret 形式 Authorization 也应被拒绝"
        );

        let mut draft = oauth_draft();
        draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap()
            .authorize_params = vec![crate::mcp::models::OAuthParam {
            key: "scope".to_string(),
            value: "read".to_string(),
        }];
        assert!(validate_draft(&draft).is_err(), "保留参数应被拒绝");

        let mut draft = oauth_draft();
        draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap()
            .authorize_params = vec![crate::mcp::models::OAuthParam {
            key: "audience".to_string(),
            value: "a\nb".to_string(),
        }];
        assert!(validate_draft(&draft).is_err(), "CR/LF 应被拒绝");
    }

    #[test]
    fn oauth_validation_accepts_cimd_and_rejects_invalid() {
        let mut draft = oauth_draft();
        draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap()
            .client_metadata_url =
            Some("https://client.example.com/oauth-client-metadata.json".to_string());
        assert!(validate_draft(&draft).is_ok(), "合法 CIMD URL 应通过");

        let mut draft = oauth_draft();
        draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap()
            .client_metadata_url = Some("http://client.example.com/metadata.json".to_string());
        assert!(validate_draft(&draft).is_err(), "CIMD 必须 HTTPS");

        let mut draft = oauth_draft();
        let oauth = draft
            .http
            .as_mut()
            .unwrap()
            .oauth
            .as_mut()
            .unwrap();
        oauth.client_metadata_url =
            Some("https://client.example.com/metadata.json".to_string());
        oauth.client_secret = Some("****".to_string());
        assert!(validate_draft(&draft).is_err(), "CIMD 不能配 secret");
    }

    #[test]
    fn oauth_draft_wire_format_uses_oauth_secret_kind() {
        // 回归：SecretKindInput 曾因 serde snake_case 把 OAuth 写成 `o_auth`，
        // 导致前端提交的 `kind: "oauth"` 在 test_server_connection / add_server 反序列化失败。
        let draft: ServerDraft = serde_json::from_value(serde_json::json!({
            "name": "oauth",
            "transport": "streamable_http",
            "stdio": null,
            "http": {
                "url": "https://example.com/mcp",
                "headers": [],
                "timeout_secs": null,
                "allow_insecure_tls": false,
                "oauth": { "scopes": [] }
            },
            "protocol_version_pin": null,
            "enabled": true,
            "tags": [],
            "secrets": [
                {
                    "kind": "oauth",
                    "key": "client_secret",
                    "action": "replace",
                    "value": "s3cret"
                }
            ]
        }))
        .expect("OAuth 草稿应能从前端 JSON 反序列化");

        assert!(matches!(draft.secrets[0].kind, SecretKindInput::OAuth));
        assert_eq!(draft.secrets[0].action, SecretAction::Replace);

        // 回写方向也必须是 oauth（与 src/types/models.ts 的类型契约一致）
        let encoded = serde_json::to_value(&draft.secrets[0]).expect("序列化失败");
        assert_eq!(encoded["kind"], "oauth");
    }

    #[test]
    fn masked_secret_detection() {
        // 前端回显的掩码形态 → 视为 keep（不写回凭据库）
        assert!(looks_like_masked_secret("Beare****w7aO"));
        assert!(looks_like_masked_secret("****"));
        assert!(looks_like_masked_secret("****（已配置）"));
        assert!(!looks_like_masked_secret("Bearer mcp_abc123"));
        assert!(!looks_like_masked_secret("sk-abcdefgh12345678"));
        assert!(!looks_like_masked_secret(""));
    }

    fn portable_state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let db = Database::open(&dir.path().join("test.db")).expect("数据库打开失败");
        db.migrate().expect("迁移失败");
        let oauth = crate::mcp::oauth::OAuthService::new(db.clone(), true);
        let state = AppState {
            db,
            manager: Arc::new(ConnectionManager::new(
                RuntimeSettings::default(),
                Some(oauth.clone()),
            )),
            oauth,
            data_dir: dir.path().to_path_buf(),
            log_dir: dir.path().join("logs"),
            portable: true,
            data_dir_source: "portable".to_string(),
            test_run: Arc::new(tokio::sync::Mutex::new(None)),
        };
        (dir, state)
    }

    fn http_draft(token: &str) -> ServerDraft {
        ServerDraft {
            name: "portable-http".to_string(),
            transport: TransportKind::StreamableHttp,
            stdio: None,
            http: Some(HttpConfig {
                url: "http://127.0.0.1:8080/mcp".to_string(),
                headers: vec![KeyValue {
                    key: "Authorization".to_string(),
                    value: Some(token.to_string()),
                    secret: true,
                }],
                timeout_secs: None,
                allow_insecure_tls: false,
                oauth: None,
            }),
            protocol_version_pin: None,
            enabled: true,
            tags: Vec::new(),
            secrets: vec![SecretInput {
                kind: SecretKindInput::Header,
                key: "Authorization".to_string(),
                action: SecretAction::Replace,
                value: Some(token.to_string()),
            }],
        }
    }

    #[tokio::test]
    async fn portable_add_and_update_keep_credentials_plaintext() {
        let (_dir, state) = portable_state();
        let created = add_server_inner(&state, http_draft("Bearer first-token"))
            .await
            .expect("新增失败");

        let loaded = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, &created.id))
            .expect("查询失败")
            .expect("服务器应存在");
        let auth = &loaded.http.as_ref().unwrap().headers[0];
        assert_eq!(auth.value.as_deref(), Some("Bearer first-token"));
        assert!(auth.secret);
        assert!(
            state
                .db
                .with_read_blocking(server_repo::list_all_credentials)
                .expect("读取凭据索引失败")
                .is_empty()
        );

        update_server_inner(
            &state,
            created.id.clone(),
            http_draft("Bearer second-token"),
        )
        .await
        .expect("更新失败");
        let loaded = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, &created.id))
            .expect("查询失败")
            .expect("服务器应存在");
        let auth = &loaded.http.as_ref().unwrap().headers[0];
        assert_eq!(auth.value.as_deref(), Some("Bearer second-token"));
        assert!(auth.secret);
        assert!(
            state
                .db
                .with_read_blocking(server_repo::list_all_credentials)
                .expect("读取凭据索引失败")
                .is_empty()
        );
    }
}
