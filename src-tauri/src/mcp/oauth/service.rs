//! OAuth 登录服务（v0.3.0）
//!
//! 不缓存 `AuthorizationManager`；每次连接/登录新建 manager，共享进程内
//! token cache、refresh token 持久化数据与每服务器刷新锁。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use oauth2::TokenResponse;
use rmcp::transport::auth::{
    AuthorizationManager, AuthorizationMetadata, AuthorizationRequest, AuthorizationSession,
    OAuthClientConfig, WWWAuthenticateParams,
};
use serde::Serialize;
use tokio::sync::{Mutex, broadcast, watch};
use url::Url;

use crate::credentials;
use crate::mcp::models::{HttpConfig, OAuthConfig, OAuthParam};
use crate::storage::Database;
use crate::storage::oauth_repo::{
    self, ClientSecretSource, OAuthCredentialRow, RegistrationKind,
};

use super::callback::{self, CallbackListener};
use super::error::{OAuthError, map_auth_error, sanitize_detail};
use super::http::McpOAuthHttpClient;
use super::identity::{FreshLoginIdentity, StoredIdentity};
use super::store::{OAuthStoreMode, OAuthTokenCache, PersistentCredentialStore, refresh_ref};

const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:33318/oauth/callback";
const FLOW_TIMEOUT_SECS: u64 = 600;

/// 连接层认证失效回调（由 `ConnectionHandle` 持有）
#[async_trait]
pub trait AuthStateSink: Send + Sync {
    async fn on_auth_required(&self, server_id: &str);
}

/// 登录事件（广播给 commands/events 转发）
#[derive(Clone, Debug, Serialize)]
pub struct OAuthEvent {
    pub server_id: String,
    pub flow_id: String,
    /// authorized / error / canceled
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 登录开始结果
#[derive(Clone, Debug, Serialize)]
pub struct OAuthLoginStart {
    pub flow_id: String,
    pub auth_url: String,
    pub redirect_uri: String,
    pub flow_timeout_secs: u64,
    pub browser_opened: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_error: Option<String>,
    /// localhost/v6 等回调提示（不阻断登录）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_warning: Option<String>,
    /// 最终授权 URL 中的 scope（去重；空 = 由 AS 决定）
    pub scopes: Vec<String>,
}

/// 退出登录结果（v0.3.1）
#[derive(Clone, Debug, Serialize)]
pub struct OAuthLogoutResult {
    pub revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 清除客户端注册结果（v0.3.1）
#[derive(Clone, Debug, Serialize)]
pub struct OAuthRegistrationClearResult {
    pub cleared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// OAuth 诊断信息（v0.3.1 P2）
#[derive(Clone, Debug, Serialize)]
pub struct OAuthDiagnostics {
    pub configured: bool,
    pub authorized: bool,
    pub refreshable: bool,
    pub client_id: Option<String>,
    pub client_metadata_url: Option<String>,
    pub registration: Option<RegistrationKind>,
    pub token_endpoint_auth_method: Option<String>,
    pub issuer: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub resource: String,
    pub scopes_supported: Vec<String>,
    pub granted_scopes: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub revocation_endpoint_auth_methods_supported: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// access token 查看结果（v0.3.1 P2）
#[derive(Clone, Debug, Serialize)]
pub struct OAuthAccessToken {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// 登录状态
#[derive(Clone, Debug, Serialize)]
pub struct OAuthStatus {
    pub configured: bool,
    pub authorized: bool,
    pub refreshable: bool,
    pub client_id: Option<String>,
    /// 实际注册方式（v0.3.1）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration: Option<RegistrationKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub issuer: Option<String>,
}

struct FlowEntry {
    flow_id: String,
    cancel: watch::Sender<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// OAuth 服务（AppState 单例）
pub struct OAuthService {
    db: Database,
    portable: bool,
    /// server_id -> flow
    flows: Arc<Mutex<HashMap<String, FlowEntry>>>,
    /// server_id -> 进程内完整凭据
    token_caches: Arc<Mutex<HashMap<String, OAuthTokenCache>>>,
    /// server_id -> 刷新锁
    refresh_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    events: broadcast::Sender<OAuthEvent>,
}

impl OAuthService {
    pub fn new(db: Database, portable: bool) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        Arc::new(Self {
            db,
            portable,
            flows: Arc::new(Mutex::new(HashMap::new())),
            token_caches: Arc::new(Mutex::new(HashMap::new())),
            refresh_locks: Arc::new(Mutex::new(HashMap::new())),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OAuthEvent> {
        self.events.subscribe()
    }

    async fn cache_for(&self, server_id: &str) -> OAuthTokenCache {
        let mut guard = self.token_caches.lock().await;
        guard
            .entry(server_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    }

    async fn refresh_lock(&self, server_id: &str) -> Arc<Mutex<()>> {
        let mut guard = self.refresh_locks.lock().await;
        guard
            .entry(server_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn load_row(&self, server_id: &str) -> Result<Option<OAuthCredentialRow>, OAuthError> {
        let server_id = server_id.to_string();
        self.db
            .with_read(move |conn| oauth_repo::get(conn, &server_id))
            .await
            .map_err(|e| OAuthError::Storage {
                message: format!("读取 OAuth 凭据失败：{e}"),
            })
    }

    async fn save_row(&self, row: OAuthCredentialRow) -> Result<(), OAuthError> {
        self.db
            .with_write(move |conn| oauth_repo::upsert(conn, &row))
            .await
            .map_err(|e| OAuthError::Storage {
                message: format!("写入 OAuth 凭据失败：{e}"),
            })
    }

    async fn clear_token_row(&self, server_id: &str) -> Result<(), OAuthError> {
        let server_id = server_id.to_string();
        self.db
            .with_write(move |conn| oauth_repo::clear_token(conn, &server_id))
            .await
            .map_err(|e| OAuthError::Storage {
                message: format!("清理 OAuth token 失败：{e}"),
            })
    }

    /// 创建 manager（连接用；有 token 时初始化并配置客户端）
    pub async fn manager_for(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        mode: OAuthStoreMode,
    ) -> Result<AuthorizationManager, OAuthError> {
        self.manager_for_inner(server_id, cfg, mode, None).await
    }

    /// 带挑战的 manager（强制刷新/诊断用）
    pub async fn manager_for_with_challenge(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        mode: OAuthStoreMode,
        challenge: Option<&str>,
    ) -> Result<AuthorizationManager, OAuthError> {
        self.manager_for_inner(server_id, cfg, mode, challenge)
            .await
    }

    async fn manager_for_inner(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        mode: OAuthStoreMode,
        challenge: Option<&str>,
    ) -> Result<AuthorizationManager, OAuthError> {
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        let row = if mode == OAuthStoreMode::Persistent {
            self.load_row(server_id).await?
        } else {
            None
        };
        let identity = StoredIdentity::resolve(oauth, row.as_ref());
        let fingerprint = config_fingerprint(
            cfg,
            identity.kind,
            identity.client_id.as_deref(),
            row.as_ref(),
        );
        let cache = if mode == OAuthStoreMode::Persistent {
            self.cache_for(server_id).await
        } else {
            Arc::new(Mutex::new(None))
        };
        let mut has_token = mode == OAuthStoreMode::Persistent
            && (cache.lock().await.is_some() || row.as_ref().is_some_and(|row| row.has_token));

        if mode == OAuthStoreMode::Persistent
            && row
                .as_ref()
                .is_some_and(|row| row.config_fingerprint.as_deref() != Some(fingerprint.as_str()))
        {
            self.clear_token_and_cache(server_id, row.as_ref()).await?;
            if identity.kind == Some(RegistrationKind::Dcr) {
                self.clear_dcr_registration(server_id).await?;
            }
            let refreshed = self.load_row(server_id).await?;
            let refreshed_identity = StoredIdentity::resolve(oauth, refreshed.as_ref());
            let refreshed_fingerprint = config_fingerprint(
                cfg,
                refreshed_identity.kind,
                refreshed_identity.client_id.as_deref(),
                refreshed.as_ref(),
            );
            self.set_fingerprint(server_id, &refreshed_fingerprint)
                .await?;
            has_token = false;
        }

        let http = McpOAuthHttpClient::new(cfg).map_err(|e| OAuthError::Config {
            message: format!("OAuth HTTP 客户端创建失败：{e}"),
        })?;
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(cfg.url.as_str(), Arc::new(http))
                .await
                .map_err(|e| OAuthError::Config {
                    message: format!("OAuth 管理器创建失败：{e}"),
                })?;
        manager.set_allow_missing_issuer(oauth.allow_missing_issuer);
        manager.set_credential_store(PersistentCredentialStore::new(
            self.db.clone(),
            self.portable,
            server_id,
            mode,
            cache,
            self.refresh_lock(server_id).await,
            identity.kind,
            fingerprint,
            identity
                .redirect_uri
                .clone()
                .unwrap_or_else(|| redirect_uri(oauth)),
        ));

        if has_token {
            let metadata = match challenge {
                Some(challenge) => manager
                    .resolve_metadata_from_challenge(Some(challenge))
                    .await,
                None => manager.resolve_metadata().await,
            }
            .map_err(|e| OAuthError::Metadata {
                message: sanitize_detail(&e.to_string()),
            })?;
            let metadata = apply_overrides(metadata.metadata, oauth);
            validate_metadata(&metadata)?;
            manager.set_metadata(metadata);
            manager
                .initialize_from_store()
                .await
                .map_err(|e| OAuthError::Materialize {
                    message: sanitize_detail(&e.to_string()),
                })?;
            self.configure_client(&mut manager, cfg, &identity, row.as_ref())
                .await?;
        }
        Ok(manager)
    }

    /// 登录用 manager：只创建 manager；store 在 begin_login 解析身份后安装
    async fn login_manager(
        &self,
        cfg: &HttpConfig,
    ) -> Result<AuthorizationManager, OAuthError> {
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        let http = McpOAuthHttpClient::new(cfg).map_err(|e| OAuthError::Config {
            message: format!("OAuth HTTP 客户端创建失败：{e}"),
        })?;
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(cfg.url.as_str(), Arc::new(http))
                .await
                .map_err(|e| OAuthError::Config {
                    message: format!("OAuth 管理器创建失败：{e}"),
                })?;
        manager.set_allow_missing_issuer(oauth.allow_missing_issuer);
        Ok(manager)
    }

    async fn configure_client(
        &self,
        manager: &mut AuthorizationManager,
        cfg: &HttpConfig,
        identity: &StoredIdentity,
        row: Option<&OAuthCredentialRow>,
    ) -> Result<(), OAuthError> {
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        if identity.kind == Some(RegistrationKind::Cimd) {
            let Some(client_id) = identity.client_id.clone() else {
                return Ok(());
            };
            let config = OAuthClientConfig::new(
                client_id,
                identity
                    .redirect_uri
                    .clone()
                    .unwrap_or_else(|| redirect_uri(oauth)),
            )
            .with_scopes(oauth.scopes.clone());
            return manager
                .configure_client(config)
                .map_err(|e| OAuthError::Config {
                    message: sanitize_detail(&e.to_string()),
                });
        }
        let Some(client_id) = identity.client_id.clone() else {
            return Ok(());
        };
        let secret = self
            .client_secret_for(row, &client_id, identity.kind)
            .await?
            .or_else(|| oauth.client_secret.clone().filter(|secret| !secret.is_empty()));
        let config = OAuthClientConfig::new(
            client_id,
            identity
                .redirect_uri
                .clone()
                .unwrap_or_else(|| redirect_uri(oauth)),
        )
        .with_scopes(oauth.scopes.clone());
        let config = match secret {
            Some(secret) => config.with_client_secret(secret),
            None => config,
        };
        manager
            .configure_client(config)
            .map_err(|e| OAuthError::Config {
                message: sanitize_detail(&e.to_string()),
            })
    }

    async fn install_store(
        &self,
        manager: &mut AuthorizationManager,
        server_id: &str,
        cfg: &HttpConfig,
        identity: &FreshLoginIdentity,
    ) -> Result<(), OAuthError> {
        let row = self.load_row(server_id).await?;
        let fingerprint = config_fingerprint(
            cfg,
            Some(identity.kind),
            identity.client_id.as_deref(),
            row.as_ref(),
        );
        manager.set_credential_store(PersistentCredentialStore::new(
            self.db.clone(),
            self.portable,
            server_id,
            OAuthStoreMode::Persistent,
            self.cache_for(server_id).await,
            self.refresh_lock(server_id).await,
            Some(identity.kind),
            fingerprint,
            identity.redirect_uri.clone(),
        ));
        Ok(())
    }

    /// 身份变化时清理旧 token/secret；只由写路径调用。
    async fn reconcile_identity(
        &self,
        server_id: &str,
        row: Option<&OAuthCredentialRow>,
        next: &FreshLoginIdentity,
    ) -> Result<(), OAuthError> {
        let Some(row) = row else {
            return Ok(());
        };
        let kind_changed = row.registration_kind != Some(next.kind);
        let client_changed = row.client_id != next.client_id;
        if !kind_changed && !client_changed {
            return Ok(());
        }

        self.clear_cache(server_id).await;
        let base = refresh_ref(server_id);
        let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);

        let mut updated = row.clone();
        updated.refresh_ref = None;
        updated.refresh_token = None;
        updated.refresh_chunks = 0;
        updated.has_token = false;
        updated.has_refresh_token = false;
        updated.expires_at = None;
        updated.scopes.clear();
        updated.issuer = None;
        updated.registration_kind = Some(next.kind);
        updated.client_id = next.client_id.clone();
        updated.redirect_uri = Some(next.redirect_uri.clone());
        updated.config_fingerprint = None;
        updated.updated_at = Utc::now();

        let keep_secret =
            next.kind != RegistrationKind::Cimd && !client_changed && row.client_id.is_some();
        if !keep_secret {
            if let Some(reference) = row.client_secret_ref.as_deref() {
                let _ = credentials::delete_secret(reference);
            }
            updated.client_secret_ref = None;
            updated.client_secret = None;
            updated.client_secret_source = None;
        } else if updated.client_secret_source.is_none()
            && (updated.client_secret.is_some() || updated.client_secret_ref.is_some())
        {
            updated.client_secret_source = Some(match next.kind {
                RegistrationKind::Dcr => ClientSecretSource::Dcr,
                RegistrationKind::Preregistered | RegistrationKind::Cimd => {
                    ClientSecretSource::User
                }
            });
        }
        self.save_row(updated).await
    }

    async fn client_secret_for(
        &self,
        row: Option<&OAuthCredentialRow>,
        client_id: &str,
        kind: Option<RegistrationKind>,
    ) -> Result<Option<String>, OAuthError> {
        let Some(row) = row else { return Ok(None) };
        if kind == Some(RegistrationKind::Cimd) {
            return Ok(None);
        }
        if kind.is_some() && row.registration_kind != kind {
            return Ok(None);
        }
        if row.client_id.as_deref() != Some(client_id) {
            return Ok(None);
        }
        if let Some(value) = row.client_secret.clone() {
            return Ok(Some(value));
        }
        let Some(reference) = row.client_secret_ref.as_deref() else {
            return Ok(None);
        };
        match credentials::get_secret(reference) {
            Ok(value) => Ok(Some(value)),
            Err(crate::core::errors::AppError::CredentialNotFound) => Ok(None),
            Err(e) => Err(OAuthError::Storage {
                message: format!("读取 client_secret 失败：{e}"),
            }),
        }
    }

    /// 开始登录；不打开浏览器（由命令层负责）
    pub async fn begin_login(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        challenge: Option<String>,
    ) -> Result<OAuthLoginStart, OAuthError> {
        self.cancel_server_flow(server_id).await;
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        let requested_redirect = redirect_uri(oauth);
        let allow_fallback = oauth.client_id.is_none();
        let listener = callback::bind(&requested_redirect, allow_fallback).await?;
        let callback_warning = listener.warning.clone();
        let redirect_uri = listener.redirect_uri.clone();
        let mut manager = self.login_manager(cfg).await?;

        let metadata = match manager
            .resolve_metadata_from_challenge(challenge.as_deref())
            .await
        {
            Ok(resolution) => apply_overrides(resolution.metadata, oauth),
            Err(discover_error) => {
                if oauth.authorization_endpoint.is_some() && oauth.token_endpoint.is_some() {
                    manual_metadata(oauth)
                } else {
                    return Err(OAuthError::Metadata {
                        message: sanitize_detail(&discover_error.to_string()),
                    });
                }
            }
        };
        validate_metadata(&metadata)?;
        let issuer = metadata.issuer.clone();
        manager.set_metadata(metadata.clone());

        let row = self.load_row(server_id).await?;
        let requested_scopes = requested_scopes(oauth, row.as_ref(), challenge.as_deref());
        let identity = FreshLoginIdentity::resolve(
            oauth,
            row.as_ref(),
            &metadata,
            &redirect_uri,
        )?;
        self.reconcile_identity(server_id, row.as_ref(), &identity)
            .await?;
        self.install_store(&mut manager, server_id, cfg, &identity)
            .await?;

        let is_cimd = identity.kind == RegistrationKind::Cimd;
        let mut cimd_url = is_cimd.then(|| identity.client_id.clone()).flatten();
        let mut client_id = if is_cimd {
            None
        } else {
            identity.client_id.clone()
        };
        let mut secret = if is_cimd {
            None
        } else if let Some(id) = client_id.clone() {
            self.client_secret_for(row.as_ref(), &id, Some(identity.kind))
                .await?
                .or_else(|| oauth.client_secret.clone().filter(|value| !value.is_empty()))
        } else {
            None
        };

        if identity.needs_registration {
            let scope_refs: Vec<&str> = requested_scopes.iter().map(String::as_str).collect();
            let config = manager
                .register_client("MCP Tester", &redirect_uri, &scope_refs)
                .await
                .map_err(map_auth_error)?;
            self.persist_registration(server_id, &config, &redirect_uri)
                .await?;
            let registered = FreshLoginIdentity {
                kind: RegistrationKind::Dcr,
                client_id: Some(config.client_id.clone()),
                redirect_uri: redirect_uri.clone(),
                needs_registration: false,
            };
            self.install_store(&mut manager, server_id, cfg, &registered)
                .await?;
            client_id = Some(config.client_id.clone());
            secret = config.client_secret.clone();
            cimd_url = None;
        }

        let mut request = AuthorizationRequest::new(redirect_uri.clone())
            .with_scopes(requested_scopes)
            .with_client_name("MCP Tester");
        if let Some(url) = cimd_url {
            request = request.with_client_metadata_url(url);
        }
        if let Some(client_id) = client_id {
            request = request.with_preregistered_client(client_id);
        }
        if let Some(secret) = secret {
            request = request.with_client_secret(secret);
        }
        let session = match AuthorizationSession::new(manager, request).await {
            Ok(session) => session,
            Err((_manager, error)) => return Err(map_auth_error(error)),
        };
        let auth_url =
            append_authorize_params(session.get_authorization_url(), &oauth.authorize_params)?;
        let scopes = scopes_from_auth_url(&auth_url);
        let flow_id = uuid::Uuid::new_v4().to_string();

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let flows = Arc::clone(&self.flows);
        let events = self.events.clone();
        let db = self.db.clone();
        let flow_server = server_id.to_string();
        let flow_id_for_task = flow_id.clone();
        let flow_id_for_compare = flow_id.clone();
        let flow_lock = self.refresh_lock(server_id).await;
        let task = tokio::spawn(async move {
            let result = run_callback_flow(
                session,
                listener,
                cancel_rx,
                db,
                flow_server.clone(),
                issuer,
                flow_lock,
            )
            .await;
            let event = match result {
                Ok(()) => OAuthEvent {
                    server_id: flow_server.clone(),
                    flow_id: flow_id_for_task,
                    state: "authorized".to_string(),
                    message: None,
                },
                Err(OAuthError::Canceled) => OAuthEvent {
                    server_id: flow_server.clone(),
                    flow_id: flow_id_for_task,
                    state: "canceled".to_string(),
                    message: None,
                },
                Err(error) => OAuthEvent {
                    server_id: flow_server.clone(),
                    flow_id: flow_id_for_task,
                    state: "error".to_string(),
                    message: Some(sanitize_detail(&error.to_string())),
                },
            };
            let _ = events.send(event);
            let mut guard = flows.lock().await;
            if guard
                .get(&flow_server)
                .is_some_and(|entry| entry.flow_id == flow_id_for_compare)
            {
                guard.remove(&flow_server);
            }
        });
        self.flows.lock().await.insert(
            server_id.to_string(),
            FlowEntry {
                flow_id: flow_id.clone(),
                cancel: cancel_tx,
                tasks: vec![task],
            },
        );

        Ok(OAuthLoginStart {
            flow_id,
            auth_url,
            redirect_uri,
            flow_timeout_secs: FLOW_TIMEOUT_SECS,
            browser_opened: false,
            open_error: None,
            callback_warning,
            scopes,
        })
    }

    async fn persist_registration(
        &self,
        server_id: &str,
        config: &OAuthClientConfig,
        redirect_uri: &str,
    ) -> Result<(), OAuthError> {
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        let old = self.load_row(server_id).await?;
        let version = old
            .as_ref()
            .map(|row| row.client_secret_version)
            .unwrap_or(0)
            + 1;
        let (secret_ref, secret_plain) =
            self.persist_client_secret(server_id, config.client_secret.as_deref())?;
        if let Some(old) = old.as_ref() {
            let base = refresh_ref(server_id);
            let _ = credentials::delete_secret_chunked(&base, old.refresh_chunks.max(0) as u32);
        }
        self.clear_cache(server_id).await;
        let row = OAuthCredentialRow {
            server_id: server_id.to_string(),
            refresh_ref: None,
            refresh_token: None,
            refresh_chunks: 0,
            client_id: Some(config.client_id.clone()),
            client_secret_ref: secret_ref,
            client_secret: secret_plain,
            client_secret_source: Some(ClientSecretSource::Dcr),
            client_secret_version: version,
            redirect_uri: Some(redirect_uri.to_string()),
            scopes: config.scopes.clone(),
            issuer: None,
            expires_at: None,
            has_token: false,
            has_refresh_token: false,
            config_fingerprint: None,
            registration_kind: Some(RegistrationKind::Dcr),
            updated_at: Utc::now(),
        };
        self.save_row(row).await
    }

    fn persist_client_secret(
        &self,
        server_id: &str,
        secret: Option<&str>,
    ) -> Result<(Option<String>, Option<String>), OAuthError> {
        let base = client_secret_ref(server_id);
        match secret {
            Some(secret) if !self.portable => {
                credentials::set_secret(&base, secret).map_err(|e| OAuthError::Storage {
                    message: format!("写入 client_secret 失败：{e}"),
                })?;
                Ok((Some(base), None))
            }
            Some(secret) => {
                let _ = credentials::delete_secret(&base);
                Ok((None, Some(secret.to_string())))
            }
            None => {
                let _ = credentials::delete_secret(&base);
                Ok((None, None))
            }
        }
    }

    /// 清除 user 预注册 secret（命令层保存时使用）
    pub async fn set_user_client_secret(
        &self,
        server_id: &str,
        client_id: Option<String>,
        secret: Option<String>,
    ) -> Result<(), OAuthError> {
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        let old = self.load_row(server_id).await?;
        let version = old
            .as_ref()
            .map(|row| row.client_secret_version)
            .unwrap_or(0)
            + 1;
        let (secret_ref, secret_plain) =
            self.persist_client_secret(server_id, secret.as_deref())?;
        let kind = if client_id.is_some() || secret.is_some() {
            Some(RegistrationKind::Preregistered)
        } else {
            None
        };
        let identity_changed = old.as_ref().is_some_and(|row| {
            row.client_id != client_id || row.registration_kind != kind
        });
        let row = OAuthCredentialRow {
            server_id: server_id.to_string(),
            refresh_ref: None,
            refresh_token: None,
            refresh_chunks: 0,
            client_id,
            client_secret_ref: secret_ref,
            client_secret: secret_plain,
            client_secret_source: secret.is_some().then_some(ClientSecretSource::User),
            client_secret_version: version,
            redirect_uri: old.as_ref().and_then(|row| row.redirect_uri.clone()),
            scopes: if identity_changed {
                Vec::new()
            } else {
                old.as_ref()
                    .map(|row| row.scopes.clone())
                    .unwrap_or_default()
            },
            issuer: if identity_changed {
                None
            } else {
                old.as_ref().and_then(|row| row.issuer.clone())
            },
            expires_at: None,
            has_token: false,
            has_refresh_token: false,
            config_fingerprint: None,
            registration_kind: kind,
            updated_at: Utc::now(),
        };
        let base = refresh_ref(server_id);
        let _ = credentials::delete_secret_chunked(
            &base,
            old.as_ref()
                .map(|row| row.refresh_chunks)
                .unwrap_or(0)
                .max(0) as u32,
        );
        self.save_row(row).await?;
        self.clear_cache(server_id).await;
        Ok(())
    }

    /// 同步读取 user client_secret（供 `load_resolved_config` 这类同步路径使用）
    pub fn user_client_secret_blocking(&self, server_id: &str) -> Option<String> {
        let row = self
            .db
            .with_read_blocking(|conn| oauth_repo::get(conn, server_id))
            .ok()
            .flatten()?;
        if row.client_secret_source != Some(ClientSecretSource::User) {
            return None;
        }
        if let Some(value) = row.client_secret {
            return Some(value);
        }
        let reference = row.client_secret_ref.as_deref()?;
        credentials::get_secret(reference).ok()
    }

    pub async fn user_client_secret(&self, server_id: &str) -> Option<String> {
        let row = self.load_row(server_id).await.ok().flatten()?;
        if row.client_secret_source != Some(ClientSecretSource::User) {
            return None;
        }
        self.client_secret_for(
            Some(&row),
            row.client_id.as_deref()?,
            Some(RegistrationKind::Preregistered),
        )
        .await
        .ok()
        .flatten()
    }

    pub async fn has_user_client_secret(&self, server_id: &str) -> bool {
        let Ok(Some(row)) = self.load_row(server_id).await else {
            return false;
        };
        row.client_secret_source == Some(ClientSecretSource::User)
            && (row.client_secret.is_some() || row.client_secret_ref.is_some())
    }

    pub async fn cancel_login(&self, flow_id: &str) -> Result<(), OAuthError> {
        let entry = {
            let mut guard = self.flows.lock().await;
            let server_id = guard
                .iter()
                .find(|(_, entry)| entry.flow_id == flow_id)
                .map(|(server_id, _)| server_id.clone());
            server_id.and_then(|server_id| guard.remove(&server_id))
        };
        if let Some(entry) = entry {
            let _ = entry.cancel.send(true);
            for task in entry.tasks {
                let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
            }
        }
        Ok(())
    }

    async fn cancel_server_flow(&self, server_id: &str) {
        let entry = self.flows.lock().await.remove(server_id);
        if let Some(entry) = entry {
            let _ = entry.cancel.send(true);
            for task in entry.tasks {
                let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
            }
        }
    }

    /// 批量摘要（列表页用；不读 keyring）
    pub async fn summaries(&self) -> HashMap<String, OAuthStatus> {
        let rows = self
            .db
            .with_read(oauth_repo::list_all)
            .await
            .unwrap_or_default();
        let caches = self.token_caches.lock().await;
        let mut result = HashMap::new();
        for row in rows {
            let cache_has_token = match caches.get(&row.server_id) {
                Some(cache) => cache
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|creds| creds.token_response.is_some()),
                None => false,
            };
            result.insert(
                row.server_id.clone(),
                OAuthStatus {
                    configured: true,
                    authorized: cache_has_token || row.has_refresh_token,
                    refreshable: row.has_refresh_token,
                    client_id: row.client_id.clone(),
                    registration: row.registration_kind,
                    client_metadata_url: None,
                    token_endpoint_auth_method: None,
                    scopes: row.scopes.clone(),
                    expires_at: row.expires_at,
                    issuer: row.issuer.clone(),
                },
            );
        }
        result
    }

    pub async fn status(&self, server_id: &str) -> Result<OAuthStatus, OAuthError> {
        let row = self.load_row(server_id).await?;
        let cache = self.token_caches.lock().await.get(server_id).cloned();
        let cache_has_token = match cache {
            Some(cache) => cache
                .lock()
                .await
                .as_ref()
                .is_some_and(|creds| creds.token_response.is_some()),
            None => false,
        };
        let refreshable = match row.as_ref() {
            Some(row) if row.has_refresh_token => {
                if self.portable {
                    row.refresh_token.is_some()
                } else if let Some(reference) = row.refresh_ref.as_deref() {
                    credentials::get_secret_chunked(reference, row.refresh_chunks.max(0) as u32)
                        .is_ok()
                } else {
                    false
                }
            }
            _ => false,
        };
        Ok(OAuthStatus {
            configured: true,
            authorized: cache_has_token || refreshable,
            refreshable,
            client_id: row.as_ref().and_then(|row| row.client_id.clone()),
            registration: row.as_ref().and_then(|row| row.registration_kind),
            client_metadata_url: None,
            token_endpoint_auth_method: None,
            scopes: row
                .as_ref()
                .map(|row| row.scopes.clone())
                .unwrap_or_default(),
            expires_at: row.as_ref().and_then(|row| row.expires_at),
            issuer: row.as_ref().and_then(|row| row.issuer.clone()),
        })
    }

    /// 带配置的状态：只用 StoredIdentity，不 discovery、不写 DB。
    pub async fn status_with_config(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
    ) -> Result<OAuthStatus, OAuthError> {
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        let row = self.load_row(server_id).await?;
        let identity = StoredIdentity::for_config(oauth, row.as_ref());
        let cache = self.token_caches.lock().await.get(server_id).cloned();
        let cache_has_token = match cache {
            Some(cache) => cache
                .lock()
                .await
                .as_ref()
                .is_some_and(|creds| creds.token_response.is_some()),
            None => false,
        };
        let refreshable = match row.as_ref() {
            Some(row) if row.has_refresh_token => {
                if self.portable {
                    row.refresh_token.is_some()
                } else if let Some(reference) = row.refresh_ref.as_deref() {
                    credentials::get_secret_chunked(reference, row.refresh_chunks.max(0) as u32)
                        .is_ok()
                } else {
                    false
                }
            }
            _ => false,
        };
        Ok(OAuthStatus {
            configured: true,
            authorized: cache_has_token || refreshable,
            refreshable,
            client_id: identity.client_id.or_else(|| row.as_ref().and_then(|row| row.client_id.clone())),
            registration: identity.kind.or_else(|| row.as_ref().and_then(|row| row.registration_kind)),
            client_metadata_url: oauth.client_metadata_url.clone(),
            token_endpoint_auth_method: oauth.token_endpoint_auth_method.clone(),
            scopes: row
                .as_ref()
                .map(|row| row.scopes.clone())
                .unwrap_or_default(),
            expires_at: row.as_ref().and_then(|row| row.expires_at),
            issuer: row.as_ref().and_then(|row| row.issuer.clone()),
        })
    }

    /// OAuth 诊断：显式 discovery，返回端点/注册方式/原始 metadata
    pub async fn diagnostics(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        challenge: Option<&str>,
    ) -> Result<OAuthDiagnostics, OAuthError> {
        let oauth = cfg.oauth.as_ref().ok_or_else(|| OAuthError::Config {
            message: "缺少 OAuth 配置".to_string(),
        })?;
        let row = self.load_row(server_id).await?;
        let identity = StoredIdentity::for_config(oauth, row.as_ref());
        let status = self.status_with_config(server_id, cfg).await?;

        let mut metadata = None;
        let mut discovery_error = None;
        if let Ok(manager) = self.login_manager(cfg).await {
            match manager.resolve_metadata_from_challenge(challenge).await {
                Ok(resolution) => {
                    let resolved = apply_overrides(resolution.metadata, oauth);
                    validate_metadata(&resolved)?;
                    metadata = Some(resolved);
                }
                Err(error) => {
                    discovery_error = Some(sanitize_detail(&error.to_string()));
                    if oauth.authorization_endpoint.is_some() && oauth.token_endpoint.is_some() {
                        metadata = Some(manual_metadata(oauth));
                    }
                }
            }
        }

        let m = metadata.as_ref();
        let field_list = |key: &str| {
            m.and_then(|metadata| metadata.additional_fields.get(key))
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let revocation_endpoint = oauth.revocation_endpoint.clone().or_else(|| {
            m.and_then(|metadata| metadata.additional_fields.get("revocation_endpoint"))
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        });
        Ok(OAuthDiagnostics {
            configured: true,
            authorized: status.authorized,
            refreshable: status.refreshable,
            client_id: identity.client_id,
            client_metadata_url: oauth.client_metadata_url.clone(),
            registration: identity.kind,
            token_endpoint_auth_method: oauth.token_endpoint_auth_method.clone(),
            issuer: m.and_then(|metadata| metadata.issuer.clone()),
            authorization_endpoint: m.map(|metadata| metadata.authorization_endpoint.clone()),
            token_endpoint: m.map(|metadata| metadata.token_endpoint.clone()),
            registration_endpoint: m.and_then(|metadata| metadata.registration_endpoint.clone()),
            revocation_endpoint,
            resource: cfg.url.clone(),
            scopes_supported: m
                .and_then(|metadata| metadata.scopes_supported.clone())
                .unwrap_or_default(),
            granted_scopes: row
                .as_ref()
                .map(|row| row.scopes.clone())
                .unwrap_or_default(),
            code_challenge_methods_supported: m
                .and_then(|metadata| metadata.code_challenge_methods_supported.clone())
                .unwrap_or_default(),
            token_endpoint_auth_methods_supported: m
                .map(|metadata| auth_methods_from_metadata(metadata, false))
                .unwrap_or_default(),
            revocation_endpoint_auth_methods_supported: field_list(
                "revocation_endpoint_auth_methods_supported",
            ),
            expires_at: row.as_ref().and_then(|row| row.expires_at),
            discovery_error,
            metadata: m.and_then(|metadata| serde_json::to_value(metadata).ok()),
        })
    }

    /// 强制刷新 token
    pub async fn force_refresh(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        challenge: Option<&str>,
    ) -> Result<OAuthStatus, OAuthError> {
        let row = self.load_row(server_id).await?;
        if !row.as_ref().is_some_and(|row| row.has_refresh_token) {
            return Err(OAuthError::NoToken);
        }
        let manager = self
            .manager_for_with_challenge(server_id, cfg, OAuthStoreMode::Persistent, challenge)
            .await?;
        manager.refresh_token().await.map_err(map_auth_error)?;
        self.status_with_config(server_id, cfg).await
    }

    /// 查看当前 access token（显式用户操作）
    pub async fn access_token(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
        challenge: Option<&str>,
    ) -> Result<OAuthAccessToken, OAuthError> {
        let manager = self
            .manager_for_with_challenge(server_id, cfg, OAuthStoreMode::Persistent, challenge)
            .await?;
        let access_token = manager.get_access_token().await.map_err(map_auth_error)?;
        let token_type = manager
            .get_credentials()
            .await
            .ok()
            .and_then(|(_, token)| token)
            .map(|token| token.token_type().as_ref().to_string())
            .unwrap_or_else(|| "Bearer".to_string());
        let expires_at = self
            .status_with_config(server_id, cfg)
            .await?
            .expires_at;
        Ok(OAuthAccessToken {
            access_token,
            token_type,
            expires_at,
        })
    }

    /// 清除客户端注册（DCR ↔ CIMD 切换用），保留 config_json
    pub async fn clear_registration(
        &self,
        server_id: &str,
    ) -> Result<OAuthRegistrationClearResult, OAuthError> {
        self.cancel_server_flow(server_id).await;
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        self.clear_cache(server_id).await;
        let row = self.load_row(server_id).await?;
        if let Some(row) = row.as_ref() {
            let base = refresh_ref(server_id);
            let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
            if let Some(reference) = row.client_secret_ref.as_deref() {
                let _ = credentials::delete_secret(reference);
            }
        }
        let server_id_owned = server_id.to_string();
        self.db
            .with_write(move |conn| oauth_repo::clear_registration(conn, &server_id_owned))
            .await
            .map_err(|e| OAuthError::Storage {
                message: format!("清除 OAuth 注册失败：{e}"),
            })?;
        Ok(OAuthRegistrationClearResult {
            cleared: true,
            detail: None,
        })
    }

    /// 退出登录：尽力撤销授权（RFC 7009），失败也清本地 token
    pub async fn logout(
        &self,
        server_id: &str,
        cfg: Option<&HttpConfig>,
        challenge: Option<&str>,
    ) -> Result<OAuthLogoutResult, OAuthError> {
        self.cancel_server_flow(server_id).await;
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        let row = self.load_row(server_id).await?;
        let mut revoked = false;
        let mut detail = None;

        if let Some(cfg) = cfg
            && let Some(oauth) = cfg.oauth.as_ref()
            && let Some(row) = row.as_ref()
        {
            let refresh_token = self.read_refresh_token(row)?;
            let access_token = self.cached_access_token(server_id).await;
            let (token, hint) = match (refresh_token, access_token) {
                (Some(token), _) => (Some(token), "refresh_token"),
                (None, Some(token)) => (Some(token), "access_token"),
                (None, None) => (None, ""),
            };
            if let Some(token) = token {
                match self
                    .revoke_token(cfg, oauth, row, challenge, &token, hint)
                    .await
                {
                    Ok(()) => revoked = true,
                    Err(message) => detail = Some(message),
                }
            } else {
                detail = Some("没有可撤销的 token".to_string());
            }
        } else if cfg.is_none() {
            detail = Some("缺少 OAuth 配置，仅清理本地凭据".to_string());
        }

        self.clear_cache(server_id).await;
        let old_chunks = row.as_ref().map(|row| row.refresh_chunks).unwrap_or(0);
        let base = refresh_ref(server_id);
        let _ = credentials::delete_secret_chunked(&base, old_chunks.max(0) as u32);
        self.clear_token_row(server_id).await?;
        Ok(OAuthLogoutResult { revoked, detail })
    }

    fn read_refresh_token(&self, row: &OAuthCredentialRow) -> Result<Option<String>, OAuthError> {
        if self.portable {
            return Ok(row.refresh_token.clone());
        }
        let Some(reference) = row.refresh_ref.as_deref() else {
            return Ok(None);
        };
        match credentials::get_secret_chunked(reference, row.refresh_chunks.max(0) as u32) {
            Ok(value) => Ok(Some(value)),
            Err(crate::core::errors::AppError::CredentialNotFound) => Ok(None),
            Err(e) => Err(OAuthError::Storage {
                message: format!("读取 refresh token 失败：{e}"),
            }),
        }
    }

    async fn cached_access_token(&self, server_id: &str) -> Option<String> {
        let cache = self.token_caches.lock().await.get(server_id).cloned()?;
        let guard = cache.lock().await;
        guard.as_ref().and_then(|credentials| {
            credentials
                .token_response
                .as_ref()
                .map(|token| token.access_token().secret().to_string())
        })
    }

    async fn revoke_token(
        &self,
        cfg: &HttpConfig,
        oauth: &OAuthConfig,
        row: &OAuthCredentialRow,
        challenge: Option<&str>,
        token: &str,
        hint: &str,
    ) -> Result<(), String> {
        let mut endpoint = oauth.revocation_endpoint.clone();
        let mut auth_methods = Vec::new();
        if endpoint.is_none() {
            if let Ok(manager) = self.login_manager(cfg).await
                && let Ok(metadata) = manager.resolve_metadata_from_challenge(challenge).await
            {
                let metadata = apply_overrides(metadata.metadata, oauth);
                endpoint = metadata
                    .additional_fields
                    .get("revocation_endpoint")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                auth_methods = auth_methods_from_metadata(&metadata, true);
            }
        } else if let Ok(manager) = self.login_manager(cfg).await
            && let Ok(metadata) = manager.resolve_metadata_from_challenge(challenge).await
        {
            let metadata = apply_overrides(metadata.metadata, oauth);
            auth_methods = auth_methods_from_metadata(&metadata, true);
        }
        let Some(endpoint) = endpoint else {
            return Err("授权服务器未提供 revocation_endpoint".to_string());
        };
        let identity = StoredIdentity::resolve(oauth, Some(row));
        let Some(client_id) = identity.client_id.clone() else {
            return Err("缺少 client_id，无法撤销".to_string());
        };
        let secret = self
            .client_secret_for(Some(row), &client_id, identity.kind)
            .await
            .map_err(|e| e.to_string())?
            .or_else(|| oauth.client_secret.clone().filter(|value| !value.is_empty()));
        let method = choose_auth_method(&auth_methods, oauth.token_endpoint_auth_method.as_deref());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .danger_accept_invalid_certs(cfg.allow_insecure_tls)
            .build()
            .map_err(|e| format!("撤销客户端创建失败：{e}"))?;
        let mut form: Vec<(String, String)> = vec![
            ("token".to_string(), token.to_string()),
            ("token_type_hint".to_string(), hint.to_string()),
        ];
        let mut builder = client.post(&endpoint);
        match (method.as_str(), secret) {
            ("client_secret_basic", Some(secret)) => {
                let encoded_id: String =
                    url::form_urlencoded::byte_serialize(client_id.as_bytes()).collect();
                let encoded_secret: String =
                    url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
                let basic = base64::engine::general_purpose::STANDARD
                    .encode(format!("{encoded_id}:{encoded_secret}"));
                builder = builder.header(
                    reqwest::header::AUTHORIZATION,
                    format!("Basic {basic}"),
                );
            }
            ("client_secret_post", Some(secret)) => {
                form.push(("client_id".to_string(), client_id));
                form.push(("client_secret".to_string(), secret));
            }
            _ => {
                form.push(("client_id".to_string(), client_id));
            }
        }
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form.iter().map(|(key, value)| (key.as_str(), value.as_str())))
            .finish();
        let response = builder
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|e| format!("撤销请求失败：{e}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("撤销端点返回 HTTP {}", response.status()))
        }
    }

    pub async fn clear(&self, server_id: &str) -> Result<(), OAuthError> {
        self.cancel_server_flow(server_id).await;
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        self.clear_cache(server_id).await;
        let row = self.load_row(server_id).await?;
        if let Some(row) = row.as_ref() {
            let base = refresh_ref(server_id);
            let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
            if let Some(reference) = row.client_secret_ref.as_deref() {
                let _ = credentials::delete_secret(reference);
            }
        }
        let server_id_owned = server_id.to_string();
        self.db
            .with_write(move |conn| oauth_repo::delete(conn, &server_id_owned))
            .await
            .map_err(|e| OAuthError::Storage {
                message: format!("删除 OAuth 凭据失败：{e}"),
            })?;
        self.token_caches.lock().await.remove(server_id);
        // 不删除 refresh lock：避免 clear 后新锁绕过与旧 refresh 的串行化
        Ok(())
    }

    pub async fn on_config_changed(
        &self,
        server_id: &str,
        cfg: &HttpConfig,
    ) -> Result<(), OAuthError> {
        let Some(oauth) = cfg.oauth.as_ref() else {
            return self.clear(server_id).await;
        };
        let lock = self.refresh_lock(server_id).await;
        let _guard = lock.lock().await;
        let Some(row) = self.load_row(server_id).await? else {
            return Ok(());
        };
        let target = StoredIdentity::for_config(oauth, Some(&row));
        let fingerprint = config_fingerprint(
            cfg,
            target.kind,
            target.client_id.as_deref(),
            Some(&row),
        );
        let token_changed = row.config_fingerprint.as_deref() != Some(fingerprint.as_str());
        let kind_changed = row.registration_kind != target.kind;
        let client_changed = row.client_id != target.client_id;
        if !token_changed && !kind_changed && !client_changed {
            return Ok(());
        }

        if token_changed {
            self.clear_cache(server_id).await;
            let base = refresh_ref(server_id);
            let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
        }

        let mut updated = row.clone();
        if kind_changed || client_changed {
            let keep_secret = target.kind != Some(RegistrationKind::Cimd)
                && row.client_id == target.client_id
                && row.client_secret_source == Some(ClientSecretSource::User);
            if !keep_secret {
                if let Some(reference) = row.client_secret_ref.as_deref() {
                    let _ = credentials::delete_secret(reference);
                }
                updated.client_secret_ref = None;
                updated.client_secret = None;
                updated.client_secret_source = None;
            } else {
                updated.client_secret_source = Some(ClientSecretSource::User);
            }
            updated.registration_kind = target.kind;
            updated.client_id = target.client_id.clone();
            updated.redirect_uri = Some(redirect_uri(oauth));
            updated.scopes.clear();
            updated.issuer = None;
            updated.expires_at = None;
            updated.client_secret_version += 1;
        }
        if token_changed {
            updated.refresh_ref = None;
            updated.refresh_token = None;
            updated.refresh_chunks = 0;
            updated.has_token = false;
            updated.has_refresh_token = false;
            updated.expires_at = None;
        }
        updated.config_fingerprint = Some(fingerprint);
        updated.updated_at = Utc::now();
        self.save_row(updated).await
    }

    async fn clear_cache(&self, server_id: &str) {
        if let Some(cache) = self.token_caches.lock().await.get(server_id).cloned() {
            *cache.lock().await = None;
        }
    }

    async fn clear_dcr_registration(&self, server_id: &str) -> Result<(), OAuthError> {
        let Some(mut updated) = self.load_row(server_id).await? else {
            return Ok(());
        };
        if let Some(reference) = updated.client_secret_ref.as_deref() {
            let _ = credentials::delete_secret(reference);
        }
        updated.client_id = None;
        updated.client_secret_ref = None;
        updated.client_secret = None;
        updated.client_secret_source = None;
        updated.registration_kind = None;
        updated.client_secret_version += 1;
        updated.config_fingerprint = None;
        updated.updated_at = Utc::now();
        self.save_row(updated).await
    }

    async fn set_fingerprint(&self, server_id: &str, fingerprint: &str) -> Result<(), OAuthError> {
        let Some(mut row) = self.load_row(server_id).await? else {
            return Ok(());
        };
        row.config_fingerprint = Some(fingerprint.to_string());
        row.updated_at = Utc::now();
        self.save_row(row).await
    }

    async fn clear_token_and_cache(
        &self,
        server_id: &str,
        row: Option<&OAuthCredentialRow>,
    ) -> Result<(), OAuthError> {
        self.clear_cache(server_id).await;
        if let Some(row) = row {
            let base = refresh_ref(server_id);
            let _ = credentials::delete_secret_chunked(&base, row.refresh_chunks.max(0) as u32);
        }
        self.clear_token_row(server_id).await
    }
}

#[async_trait]
impl AuthStateSink for OAuthService {
    async fn on_auth_required(&self, server_id: &str) {
        // 只清进程内 access token；refresh token 由
        // PersistentCredentialStore::on_refresh_token_rejected 的 CAS 清理，
        // 避免旧请求的 401 清掉刚登录的新 token。
        self.clear_cache(server_id).await;
    }
}

async fn run_callback_flow(
    session: AuthorizationSession,
    listener: CallbackListener,
    cancel: watch::Receiver<bool>,
    db: Database,
    server_id: String,
    issuer: Option<String>,
    lock: Arc<Mutex<()>>,
) -> Result<(), OAuthError> {
    let callback_url = callback::wait_for_callback(listener, cancel.clone(), None).await?;
    let token = session
        .handle_callback_url(&callback_url)
        .await
        .map_err(map_auth_error)?;
    let _guard = lock.lock().await;
    if *cancel.borrow() {
        // 只清本次 exchange 写入的 token，避免清掉随后新登录的结果
        let expected = token
            .refresh_token()
            .map(|token| token.secret().to_string());
        let server = server_id.clone();
        db.with_write(move |conn| {
            let Some(row) = oauth_repo::get(conn, &server)? else {
                return Ok(());
            };
            let current = if let Some(value) = row.refresh_token.clone() {
                Some(value)
            } else if let Some(reference) = row.refresh_ref.as_deref() {
                credentials::get_secret_chunked(
                    reference,
                    row.refresh_chunks.max(0) as u32,
                )
                .ok()
            } else {
                None
            };
            if expected.is_some() && current == expected {
                let _ = credentials::delete_secret_chunked(
                    &refresh_ref(&server),
                    row.refresh_chunks.max(0) as u32,
                );
                oauth_repo::clear_token(conn, &server)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| OAuthError::Storage {
            message: format!("清理取消的 OAuth token 失败：{e}"),
        })?;
        return Err(OAuthError::Canceled);
    }
    let now = Utc::now();
    let server_id_for_db = server_id.clone();
    db.with_write(move |conn| {
        if let Some(mut row) = oauth_repo::get(conn, &server_id_for_db)? {
            row.redirect_uri = Some(session.redirect_uri.clone());
            // token 响应可以省略 scope；此时 credential store 已写入
            // requested/granted scopes，不能用空数组覆盖。
            if let Some(scopes) = token.scopes() {
                row.scopes = scopes
                    .iter()
                    .map(|scope| scope.as_ref().to_string())
                    .collect();
            }
            row.issuer = issuer;
            row.expires_at = token.expires_in().and_then(|expires| {
                chrono::Duration::from_std(expires)
                    .ok()
                    .map(|duration| now + duration)
            });
            row.has_token = true;
            row.has_refresh_token = token.refresh_token().is_some();
            row.updated_at = now;
            oauth_repo::upsert(conn, &row)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| OAuthError::Storage {
        message: format!("更新 OAuth 登录状态失败：{e}"),
    })?;
    Ok(())
}

fn redirect_uri(oauth: &OAuthConfig) -> String {
    oauth
        .redirect_uri
        .clone()
        .unwrap_or_else(|| DEFAULT_REDIRECT_URI.to_string())
}

fn client_secret_ref(server_id: &str) -> String {
    format!("server/{server_id}/oauth/client_secret")
}

/// 保留参数：由 rmcp/PKCE/客户端身份生成，禁止用户覆盖。
fn is_reserved_authorize_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "state"
            | "code_challenge"
            | "code_challenge_method"
            | "client_id"
            | "redirect_uri"
            | "response_type"
            | "scope"
            | "resource"
    )
}

/// 影响 token 身份的参数；prompt/login_hint 等只影响交互，不触发重新登录。
fn is_identity_authorize_param(key: &str) -> bool {
    !matches!(
        key.to_ascii_lowercase().as_str(),
        "prompt" | "login_hint" | "ui_locales" | "display" | "max_age"
    )
}

fn append_authorize_params(url: &str, params: &[OAuthParam]) -> Result<String, OAuthError> {
    if params.is_empty() {
        return Ok(url.to_string());
    }
    let mut parsed = Url::parse(url).map_err(|e| OAuthError::Config {
        message: format!("授权 URL 解析失败：{e}"),
    })?;
    let existing: std::collections::HashSet<String> = parsed
        .query_pairs()
        .map(|(key, _)| key.to_ascii_lowercase())
        .collect();
    for param in params {
        let key = param.key.trim();
        if key.is_empty() {
            return Err(OAuthError::Config {
                message: "授权请求参数 key 不能为空".to_string(),
            });
        }
        if is_reserved_authorize_key(key) {
            return Err(OAuthError::Config {
                message: format!("授权请求参数「{key}」是保留参数"),
            });
        }
        if existing.contains(&key.to_ascii_lowercase()) {
            return Err(OAuthError::Config {
                message: format!("授权请求参数「{key}」与现有参数冲突"),
            });
        }
    }
    {
        let mut pairs = parsed.query_pairs_mut();
        for param in params {
            pairs.append_pair(param.key.trim(), &param.value);
        }
    }
    Ok(parsed.to_string())
}

/// 请求 scope = oauth.scopes ∪ row.scopes ∪ challenge.scope（去重保序）。
fn requested_scopes(
    oauth: &OAuthConfig,
    row: Option<&OAuthCredentialRow>,
    challenge: Option<&str>,
) -> Vec<String> {
    let mut scopes = oauth.scopes.clone();
    if let Some(row) = row {
        scopes.extend(row.scopes.iter().cloned());
    }
    if let Some(challenge) = challenge {
        let base = Url::parse("http://localhost/").expect("固定 URL 解析失败");
        if let Some(scope) = WWWAuthenticateParams::parse(challenge, &base).scope {
            scopes.extend(scope.split_whitespace().map(str::to_string));
        }
    }
    let mut seen = std::collections::HashSet::new();
    scopes
        .into_iter()
        .filter(|scope| !scope.is_empty() && seen.insert(scope.clone()))
        .collect()
}

fn scopes_from_auth_url(url: &str) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else {
        return Vec::new();
    };
    let mut scopes = Vec::new();
    for (key, value) in parsed.query_pairs() {
        if key != "scope" {
            continue;
        }
        for scope in value.split_whitespace() {
            if !scopes.iter().any(|existing| existing == scope) {
                scopes.push(scope.to_string());
            }
        }
    }
    scopes
}

fn config_fingerprint(
    cfg: &HttpConfig,
    kind: Option<RegistrationKind>,
    client_id: Option<&str>,
    row: Option<&OAuthCredentialRow>,
) -> String {
    let Some(oauth) = cfg.oauth.as_ref() else {
        return String::new();
    };
    let mut scopes = oauth.scopes.clone();
    scopes.sort();
    scopes.dedup();
    let old = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        cfg.url,
        client_id.unwrap_or_default(),
        redirect_uri(oauth),
        scopes.join(" "),
        oauth.authorization_endpoint.as_deref().unwrap_or_default(),
        oauth.token_endpoint.as_deref().unwrap_or_default(),
        oauth.registration_endpoint.as_deref().unwrap_or_default(),
        oauth.issuer.as_deref().unwrap_or_default(),
        oauth.allow_missing_issuer,
        cfg.allow_insecure_tls,
        row.map(|row| row.client_secret_version).unwrap_or(0),
    );
    let mut identity_params: Vec<(String, String)> = oauth
        .authorize_params
        .iter()
        .filter(|param| is_identity_authorize_param(&param.key))
        .map(|param| (param.key.to_ascii_lowercase(), param.value.clone()))
        .collect();
    let cimd = kind == Some(RegistrationKind::Cimd);
    if identity_params.is_empty() && !cimd {
        return old;
    }
    identity_params.sort_by(|a, b| a.0.cmp(&b.0));
    let kind = kind.map(RegistrationKind::as_str).unwrap_or("unknown");
    let params = serde_json::to_string(&identity_params).unwrap_or_default();
    format!(
        "v2|{}|{}|{}|{}",
        kind,
        client_id.unwrap_or_default(),
        old,
        params
    )
}

fn auth_methods_from_metadata(
    metadata: &AuthorizationMetadata,
    prefer_revocation: bool,
) -> Vec<String> {
    let read = |key: &str| {
        metadata
            .additional_fields
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    if prefer_revocation {
        let revocation = read("revocation_endpoint_auth_methods_supported");
        if !revocation.is_empty() {
            return revocation;
        }
    }
    read("token_endpoint_auth_methods_supported")
}

fn choose_auth_method(methods: &[String], configured: Option<&str>) -> String {
    if let Some(configured) = configured {
        return configured.to_string();
    }
    if methods.iter().any(|method| method == "client_secret_basic") {
        return "client_secret_basic".to_string();
    }
    if methods.iter().any(|method| method == "client_secret_post") {
        return "client_secret_post".to_string();
    }
    if methods.iter().any(|method| method == "none") {
        return "none".to_string();
    }
    "client_secret_basic".to_string()
}

fn apply_overrides(
    mut metadata: AuthorizationMetadata,
    oauth: &OAuthConfig,
) -> AuthorizationMetadata {
    if let Some(value) = &oauth.authorization_endpoint {
        metadata.authorization_endpoint = value.clone();
    }
    if let Some(value) = &oauth.token_endpoint {
        metadata.token_endpoint = value.clone();
    }
    if let Some(value) = &oauth.registration_endpoint {
        metadata.registration_endpoint = Some(value.clone());
    }
    if let Some(value) = &oauth.issuer {
        metadata.issuer = Some(value.clone());
    }
    metadata
}

fn manual_metadata(oauth: &OAuthConfig) -> AuthorizationMetadata {
    let mut metadata = AuthorizationMetadata::default();
    metadata.authorization_endpoint = oauth.authorization_endpoint.clone().unwrap_or_default();
    metadata.token_endpoint = oauth.token_endpoint.clone().unwrap_or_default();
    metadata.registration_endpoint = oauth.registration_endpoint.clone();
    metadata.issuer = oauth.issuer.clone();
    metadata.response_types_supported = Some(vec!["code".to_string()]);
    metadata.code_challenge_methods_supported = Some(vec!["S256".to_string()]);
    if oauth.client_metadata_url.is_some() {
        // 用户显式配置 CIMD 时，手工端点模式无需 AS 再广告该能力
        metadata.additional_fields.insert(
            "client_id_metadata_document_supported".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if let Some(method) = oauth.token_endpoint_auth_method.as_deref() {
        metadata.additional_fields.insert(
            "token_endpoint_auth_methods_supported".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String(method.to_string())]),
        );
    }
    metadata
}

fn validate_metadata(metadata: &AuthorizationMetadata) -> Result<(), OAuthError> {
    if metadata.authorization_endpoint.is_empty() || metadata.token_endpoint.is_empty() {
        return Err(OAuthError::Metadata {
            message: "授权端点或 token 端点缺失".to_string(),
        });
    }
    ensure_secure_url(&metadata.authorization_endpoint)?;
    ensure_secure_url(&metadata.token_endpoint)?;
    if let Some(registration) = metadata.registration_endpoint.as_deref() {
        ensure_secure_url(registration)?;
    }
    Ok(())
}

fn ensure_secure_url(value: &str) -> Result<(), OAuthError> {
    let url = Url::parse(value).map_err(|e| OAuthError::Metadata {
        message: format!("端点 URL 非法：{e}"),
    })?;
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
        return Err(OAuthError::Metadata {
            message: format!("OAuth 端点必须是 https（loopback 除外）：{value}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{HttpConfig, OAuthConfig};
    use serial_test::serial;

    fn temp_state() -> (tempfile::TempDir, Arc<OAuthService>) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let db = Database::open(&dir.path().join("test.db")).expect("数据库打开失败");
        db.migrate().expect("迁移失败");
        db.with_write_blocking(|conn| {
            conn.execute(
                "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
                 VALUES ('s1', 'demo', 'streamable_http', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )?;
            Ok(())
        })
        .expect("服务器插入失败");
        (dir, OAuthService::new(db, false))
    }

    fn oauth_http(client_id: Option<&str>) -> HttpConfig {
        HttpConfig {
            url: "https://example.com/mcp".to_string(),
            headers: Vec::new(),
            timeout_secs: None,
            allow_insecure_tls: false,
            oauth: Some(OAuthConfig {
                client_id: client_id.map(ToString::to_string),
                ..Default::default()
            }),
        }
    }

    #[tokio::test]
    #[serial]
    async fn user_secret_survives_logout_and_clears_on_client_change() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, service) = temp_state();
        service
            .set_user_client_secret(
                "s1",
                Some("client-a".to_string()),
                Some("secret-a".to_string()),
            )
            .await
            .expect("保存 secret 失败");
        assert!(service.has_user_client_secret("s1").await);
        assert_eq!(
            service.user_client_secret_blocking("s1").as_deref(),
            Some("secret-a")
        );

        service
            .logout("s1", None, None)
            .await
            .expect("logout 失败");
        assert!(
            service.has_user_client_secret("s1").await,
            "logout 应保留 secret"
        );

        service
            .on_config_changed("s1", &oauth_http(Some("client-b")))
            .await
            .expect("配置变化失败");
        assert!(
            !service.has_user_client_secret("s1").await,
            "client_id 变化应清理旧 secret"
        );
    }

    #[tokio::test]
    #[serial]
    async fn config_change_clears_stale_token() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, service) = temp_state();
        let base = crate::mcp::oauth::store::refresh_ref("s1");
        credentials::set_secret_chunked(&base, "refresh-secret").expect("写入失败");
        service
            .db
            .with_write_blocking(|conn| {
                oauth_repo::upsert(
                    conn,
                    &OAuthCredentialRow {
                        server_id: "s1".to_string(),
                        refresh_ref: Some(base.clone()),
                        refresh_token: None,
                        refresh_chunks: 1,
                        client_id: Some("client-a".to_string()),
                        client_secret_ref: None,
                        client_secret: None,
                        client_secret_source: Some(ClientSecretSource::User),
                        client_secret_version: 0,
                        redirect_uri: Some("http://127.0.0.1:33318/oauth/callback".to_string()),
                        scopes: Vec::new(),
                        issuer: None,
                        expires_at: None,
                        has_token: true,
                        has_refresh_token: true,
                        config_fingerprint: Some("stale".to_string()),
                        registration_kind: Some(RegistrationKind::Preregistered),
                        updated_at: Utc::now(),
                    },
                )
            })
            .expect("写入行失败");

        service
            .on_config_changed(
                "s1",
                &HttpConfig {
                    url: "https://changed.example/mcp".to_string(),
                    headers: Vec::new(),
                    timeout_secs: None,
                    allow_insecure_tls: false,
                    oauth: Some(OAuthConfig {
                        client_id: Some("client-a".to_string()),
                        ..Default::default()
                    }),
                },
            )
            .await
            .expect("配置变化失败");

        let row = service
            .db
            .with_read_blocking(|conn| oauth_repo::get(conn, "s1"))
            .expect("查询失败")
            .expect("行不存在");
        assert!(!row.has_token);
        assert!(!row.has_refresh_token);
        assert!(row.refresh_ref.is_none());
        assert!(credentials::get_secret_chunked(&base, 1).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn manager_without_token_does_not_require_metadata() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, service) = temp_state();
        let manager = service
            .manager_for("s1", &oauth_http(None), OAuthStoreMode::InMemory)
            .await;
        assert!(manager.is_ok(), "无 token 时应返回未认证 manager");
    }

    #[test]
    fn authorize_params_append_and_reject_conflicts() {
        let url = "https://auth.example.com/authorize?client_id=abc&scope=read";
        let params = vec![OAuthParam {
            key: "audience".to_string(),
            value: "https://api.example.com".to_string(),
        }];
        let result = append_authorize_params(url, &params).expect("追加失败");
        assert!(result.contains("audience=https%3A%2F%2Fapi.example.com"));

        let conflict = vec![OAuthParam {
            key: "client_id".to_string(),
            value: "other".to_string(),
        }];
        assert!(append_authorize_params(url, &conflict).is_err());
    }

    #[test]
    fn authorize_params_fingerprint_only_identity_params() {
        let mut http = oauth_http(None);
        http.oauth.as_mut().expect("oauth").authorize_params = vec![OAuthParam {
            key: "prompt".to_string(),
            value: "consent".to_string(),
        }];
        let old = config_fingerprint(&http, None, None, None);
        assert!(!old.starts_with("v2|"), "UX 参数不应触发 v2 指纹");

        http.oauth
            .as_mut()
            .expect("oauth")
            .authorize_params
            .push(OAuthParam {
                key: "audience".to_string(),
                value: "https://api.example.com".to_string(),
            });
        let v2 = config_fingerprint(&http, Some(RegistrationKind::Dcr), None, None);
        assert!(v2.starts_with("v2|dcr|"), "身份参数应触发 v2 指纹");
    }

    #[test]
    fn requested_scopes_unions_config_row_and_challenge() {
        let oauth = OAuthConfig {
            scopes: vec!["read".to_string()],
            ..Default::default()
        };
        let row = OAuthCredentialRow {
            server_id: "s1".to_string(),
            refresh_ref: None,
            refresh_token: None,
            refresh_chunks: 0,
            client_id: None,
            client_secret_ref: None,
            client_secret: None,
            client_secret_source: None,
            client_secret_version: 0,
            redirect_uri: None,
            scopes: vec!["read".to_string(), "write".to_string()],
            issuer: None,
            expires_at: None,
            has_token: true,
            has_refresh_token: true,
            config_fingerprint: None,
            registration_kind: None,
            updated_at: Utc::now(),
        };
        let scopes = requested_scopes(
            &oauth,
            Some(&row),
            Some("Bearer error=\"insufficient_scope\", scope=\"admin write\""),
        );
        assert_eq!(scopes, vec!["read", "write", "admin"]);
    }

    #[test]
    fn scopes_from_auth_url_deduplicates() {
        let url = "https://auth.example.com/authorize?scope=read+write&scope=read";
        assert_eq!(
            scopes_from_auth_url(url),
            vec!["read".to_string(), "write".to_string()]
        );
    }
}
