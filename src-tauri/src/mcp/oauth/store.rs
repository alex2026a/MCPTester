//! refresh-only OAuth 凭据存储（v0.3.0）
//!
//! - access token 只保留在进程内 `OAuthTokenCache`；
//! - 持久化只写 refresh token（安装版 keyring 分块 / 便携版 SQLite）；
//! - 重启后合成“已过期”的 token response，触发 rmcp 自动 refresh。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use oauth2::{AccessToken, RefreshToken, TokenResponse, basic::BasicTokenType};
use rmcp::transport::auth::{
    AuthError, CredentialRefreshGuard, CredentialStore, OAuthTokenResponse, StoredCredentials,
    VendorExtraTokenFields,
};
use tokio::sync::Mutex;

use crate::credentials;
use crate::storage::Database;
use crate::storage::oauth_repo;

/// 进程内完整凭据缓存（含 access token）
pub type OAuthTokenCache = Arc<Mutex<Option<StoredCredentials>>>;

/// 存储模式：持久化 / 仅内存（测试连接、未保存草稿）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthStoreMode {
    Persistent,
    InMemory,
}

/// refresh token 的 keyring 基础键
pub fn refresh_ref(server_id: &str) -> String {
    format!("server/{server_id}/oauth/refresh")
}

/// 安装版 / 便携版 OAuth 凭据存储
pub struct PersistentCredentialStore {
    db: Database,
    portable: bool,
    server_id: String,
    mode: OAuthStoreMode,
    cache: OAuthTokenCache,
    refresh_guard: Arc<Mutex<()>>,
    registration_kind: Option<oauth_repo::RegistrationKind>,
    config_fingerprint: String,
    redirect_uri: String,
}

impl PersistentCredentialStore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Database,
        portable: bool,
        server_id: impl Into<String>,
        mode: OAuthStoreMode,
        cache: OAuthTokenCache,
        refresh_guard: Arc<Mutex<()>>,
        registration_kind: Option<oauth_repo::RegistrationKind>,
        config_fingerprint: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            db,
            portable,
            server_id: server_id.into(),
            mode,
            cache,
            refresh_guard,
            registration_kind,
            config_fingerprint: config_fingerprint.into(),
            redirect_uri: redirect_uri.into(),
        }
    }

    fn store_error(message: impl Into<String>) -> AuthError {
        AuthError::CredentialStoreError(message.into())
    }

    async fn read_row(&self) -> Result<Option<oauth_repo::OAuthCredentialRow>, AuthError> {
        let server_id = self.server_id.clone();
        self.db
            .with_read(move |conn| oauth_repo::get(conn, &server_id))
            .await
            .map_err(|e| Self::store_error(format!("读取 OAuth 凭据失败：{e}")))
    }

    async fn write_row(&self, row: oauth_repo::OAuthCredentialRow) -> Result<(), AuthError> {
        self.db
            .with_write(move |conn| oauth_repo::upsert(conn, &row))
            .await
            .map_err(|e| Self::store_error(format!("写入 OAuth 凭据失败：{e}")))
    }

    fn persist_refresh(
        &self,
        token: Option<&str>,
        old_chunks: i64,
    ) -> Result<(Option<String>, Option<String>, i64), AuthError> {
        let base_ref = refresh_ref(&self.server_id);
        if self.portable {
            // 便携版：明文列；清掉可能残留的 keyring ref
            if old_chunks > 0 || self.mode == OAuthStoreMode::Persistent {
                let _ = credentials::delete_secret_chunked(&base_ref, old_chunks.max(0) as u32);
            }
            return Ok((None, token.map(ToString::to_string), 0));
        }
        match token {
            Some(value) => {
                let chunks = credentials::set_secret_chunked(&base_ref, value)
                    .map_err(|e| Self::store_error(format!("写入 refresh token 失败：{e}")))?;
                if old_chunks > 0 {
                    // 新 token 变短时清理多余旧块
                    for index in chunks..old_chunks as u32 {
                        let _ = credentials::delete_secret(&format!("{base_ref}/{index}"));
                    }
                }
                Ok((Some(base_ref), None, i64::from(chunks)))
            }
            None => {
                credentials::delete_secret_chunked(&base_ref, old_chunks.max(0) as u32)
                    .map_err(|e| Self::store_error(format!("删除 refresh token 失败：{e}")))?;
                Ok((None, None, 0))
            }
        }
    }

    async fn clear_token_fields(&self) -> Result<(), AuthError> {
        let server_id = self.server_id.clone();
        self.db
            .with_write(move |conn| oauth_repo::clear_token(conn, &server_id))
            .await
            .map_err(|e| Self::store_error(format!("清理 OAuth token 失败：{e}")))?;
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for PersistentCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        if let Some(cached) = self.cache.lock().await.clone() {
            return Ok(Some(cached));
        }
        if self.mode == OAuthStoreMode::InMemory {
            return Ok(None);
        }
        let Some(row) = self.read_row().await? else {
            return Ok(None);
        };
        if !row.has_token {
            return Ok(None);
        }
        let refresh_token = if self.portable {
            row.refresh_token.clone()
        } else if let Some(reference) = row.refresh_ref.as_deref() {
            match credentials::get_secret_chunked(reference, row.refresh_chunks.max(0) as u32) {
                Ok(value) => Some(value),
                Err(crate::core::errors::AppError::CredentialNotFound) => None,
                Err(e) => return Err(Self::store_error(format!("读取 refresh token 失败：{e}"))),
            }
        } else {
            None
        };
        let Some(refresh_token) = refresh_token else {
            // 无 refresh token：重启后必须重新登录
            return Ok(None);
        };

        let mut token = OAuthTokenResponse::new(
            AccessToken::new("expired".to_string()),
            BasicTokenType::Bearer,
            VendorExtraTokenFields::default(),
        );
        token.set_refresh_token(Some(RefreshToken::new(refresh_token)));
        token.set_expires_in(Some(&Duration::from_secs(1)));

        let credentials = StoredCredentials::new(
            row.client_id.unwrap_or_default(),
            Some(token),
            row.scopes,
            Some(0),
        )
        .with_issuer(row.issuer);
        Ok(Some(credentials))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        if credentials.token_response.is_none() {
            *self.cache.lock().await = None;
            if self.mode == OAuthStoreMode::Persistent {
                let old_chunks = self
                    .read_row()
                    .await?
                    .map(|row| row.refresh_chunks)
                    .unwrap_or(0);
                let base_ref = refresh_ref(&self.server_id);
                let _ = credentials::delete_secret_chunked(&base_ref, old_chunks.max(0) as u32);
                self.clear_token_fields().await?;
            }
            return Ok(());
        }

        *self.cache.lock().await = Some(credentials.clone());
        if self.mode == OAuthStoreMode::InMemory {
            return Ok(());
        }

        let refresh_token = credentials
            .token_response
            .as_ref()
            .and_then(|token| token.refresh_token())
            .map(|token| token.secret().to_string());
        let old = self.read_row().await?;
        let old_chunks = old.as_ref().map(|row| row.refresh_chunks).unwrap_or(0);
        let (refresh_ref_value, refresh_plain, chunks) =
            self.persist_refresh(refresh_token.as_deref(), old_chunks)?;

        let now = Utc::now();
        let expires_at = credentials.token_response.as_ref().and_then(|token| {
            let expires_in = token.expires_in()?;
            let received = credentials
                .token_received_at
                .and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0))
                .unwrap_or(now);
            Some(received + chrono::Duration::from_std(expires_in).ok()?)
        });
        let same_identity = old.as_ref().is_some_and(|row| {
            row.registration_kind == self.registration_kind
                && row.client_id.as_deref() == Some(credentials.client_id.as_str())
        });
        let cimd = self.registration_kind == Some(oauth_repo::RegistrationKind::Cimd);
        if (!same_identity || cimd)
            && let Some(reference) = old
                .as_ref()
                .and_then(|row| row.client_secret_ref.as_deref())
        {
            let _ = credentials::delete_secret(reference);
        }
        let keep_secret = same_identity && !cimd;
        let (secret_ref, secret_plain, secret_source) = if keep_secret {
            (
                old.as_ref().and_then(|row| row.client_secret_ref.clone()),
                old.as_ref().and_then(|row| row.client_secret.clone()),
                old.as_ref().and_then(|row| row.client_secret_source),
            )
        } else {
            (None, None, None)
        };
        let registered_redirect = if same_identity {
            old.as_ref().and_then(|row| row.redirect_uri.clone())
        } else {
            None
        };
        let row = oauth_repo::OAuthCredentialRow {
            server_id: self.server_id.clone(),
            refresh_ref: refresh_ref_value,
            refresh_token: refresh_plain,
            refresh_chunks: chunks,
            client_id: Some(credentials.client_id.clone()),
            client_secret_ref: secret_ref,
            client_secret: secret_plain,
            client_secret_source: secret_source,
            client_secret_version: old
                .as_ref()
                .map(|row| row.client_secret_version)
                .unwrap_or(0),
            // refresh 保存不能覆盖已登记的实际回调端口
            redirect_uri: registered_redirect.or_else(|| Some(self.redirect_uri.clone())),
            scopes: credentials.granted_scopes.clone(),
            issuer: credentials.issuer.clone(),
            expires_at,
            has_token: true,
            has_refresh_token: refresh_token.is_some(),
            config_fingerprint: Some(self.config_fingerprint.clone()),
            registration_kind: self.registration_kind,
            updated_at: now,
        };
        if let Err(e) = self.write_row(row).await {
            // 回滚 keyring 写入，避免孤儿
            let base_ref = refresh_ref(&self.server_id);
            let _ = credentials::delete_secret_chunked(&base_ref, chunks.max(0) as u32);
            return Err(e);
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        *self.cache.lock().await = None;
        if self.mode == OAuthStoreMode::InMemory {
            return Ok(());
        }
        let old_chunks = self
            .read_row()
            .await?
            .map(|row| row.refresh_chunks)
            .unwrap_or(0);
        let base_ref = refresh_ref(&self.server_id);
        credentials::delete_secret_chunked(&base_ref, old_chunks.max(0) as u32)
            .map_err(|e| Self::store_error(format!("删除 refresh token 失败：{e}")))?;
        self.clear_token_fields().await
    }

    async fn on_refresh_token_rejected(
        &self,
        credentials: &StoredCredentials,
    ) -> Result<(), AuthError> {
        // compare-and-swap：只有当前存储仍是这次刷新用的 refresh token 才清，
        // 避免旧刷新失败清掉新登录的 token。
        let rejected = credentials
            .token_response
            .as_ref()
            .and_then(|token| token.refresh_token())
            .map(|token| token.secret().to_string());
        let current = self.load().await?;
        let current_token = current
            .as_ref()
            .and_then(|stored| stored.token_response.as_ref())
            .and_then(|token| token.refresh_token())
            .map(|token| token.secret().to_string());
        if rejected != current_token {
            return Ok(());
        }
        self.clear().await
    }

    async fn acquire_refresh_guard(&self) -> Result<Option<CredentialRefreshGuard>, AuthError> {
        let guard = self.refresh_guard.clone().lock_owned().await;
        Ok(Some(CredentialRefreshGuard::new(guard)))
    }
}

/// 计算 token 过期时间（供 status / 测试）
pub fn expires_at_of(
    token: &OAuthTokenResponse,
    received_at: Option<u64>,
) -> Option<DateTime<Utc>> {
    let expires_in = token.expires_in()?;
    let received = received_at
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0))
        .unwrap_or_else(Utc::now);
    Some(received + chrono::Duration::from_std(expires_in).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oauth2::{AccessToken, RefreshToken, basic::BasicTokenType};
    use rmcp::transport::auth::VendorExtraTokenFields;
    use serial_test::serial;

    fn temp_db() -> (tempfile::TempDir, Database) {
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
        (dir, db)
    }

    fn credentials(refresh: &str) -> StoredCredentials {
        let mut token = OAuthTokenResponse::new(
            AccessToken::new("access".to_string()),
            BasicTokenType::Bearer,
            VendorExtraTokenFields::default(),
        );
        token.set_refresh_token(Some(RefreshToken::new(refresh.to_string())));
        token.set_expires_in(Some(&Duration::from_secs(3600)));
        StoredCredentials::new(
            "client-1".to_string(),
            Some(token),
            vec!["read".to_string()],
            Some(Utc::now().timestamp() as u64),
        )
    }

    fn make_store(db: &Database, cache: OAuthTokenCache) -> PersistentCredentialStore {
        PersistentCredentialStore::new(
            db.clone(),
            false,
            "s1",
            OAuthStoreMode::Persistent,
            cache,
            Arc::new(Mutex::new(())),
            Some(oauth_repo::RegistrationKind::Preregistered),
            "fp-1",
            "http://127.0.0.1:33318/oauth/callback",
        )
    }

    #[tokio::test]
    #[serial]
    async fn save_load_and_restart_refresh() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, db) = temp_db();
        let cache: OAuthTokenCache = Arc::new(Mutex::new(None));
        let store = make_store(&db, cache.clone());
        store
            .save(credentials("refresh-1"))
            .await
            .expect("保存失败");
        let loaded = store.load().await.expect("读取失败").expect("无凭据");
        assert_eq!(
            loaded
                .token_response
                .as_ref()
                .and_then(|token| token.refresh_token())
                .map(|token| token.secret().as_str()),
            Some("refresh-1")
        );

        // 模拟重启：新 cache，同一 DB / keyring
        let restarted = make_store(&db, Arc::new(Mutex::new(None)));
        let synthetic = restarted
            .load()
            .await
            .expect("重启读取失败")
            .expect("无凭据");
        let token = synthetic.token_response.as_ref().expect("无 token");
        assert_eq!(
            token.refresh_token().map(|t| t.secret().as_str()),
            Some("refresh-1")
        );
        assert_eq!(token.expires_in(), Some(Duration::from_secs(1)));

        store.clear().await.expect("清理失败");
        assert!(store.load().await.expect("读取失败").is_none());
    }

    #[tokio::test]
    #[serial]
    async fn portable_mode_writes_plaintext_column() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, db) = temp_db();
        let cache: OAuthTokenCache = Arc::new(Mutex::new(None));
        let store = PersistentCredentialStore::new(
            db.clone(),
            true,
            "s1",
            OAuthStoreMode::Persistent,
            cache,
            Arc::new(Mutex::new(())),
            Some(oauth_repo::RegistrationKind::Preregistered),
            "fp-1",
            "http://127.0.0.1:33318/oauth/callback",
        );
        store
            .save(credentials("refresh-portable"))
            .await
            .expect("保存失败");
        let row = db
            .with_read_blocking(|conn| oauth_repo::get(conn, "s1"))
            .expect("查询失败")
            .expect("行不存在");
        assert_eq!(row.refresh_token.as_deref(), Some("refresh-portable"));
        assert!(row.refresh_ref.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn refresh_guard_serializes() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, db) = temp_db();
        let store = make_store(&db, Arc::new(Mutex::new(None)));
        let first = store
            .acquire_refresh_guard()
            .await
            .expect("获取 guard 失败")
            .expect("guard 缺失");
        let second =
            tokio::time::timeout(Duration::from_millis(50), store.acquire_refresh_guard()).await;
        assert!(second.is_err(), "第二个 guard 应被阻塞");
        drop(first);
        assert!(
            store
                .acquire_refresh_guard()
                .await
                .expect("获取 guard 失败")
                .is_some()
        );
    }

    #[tokio::test]
    #[serial]
    async fn rejected_compare_and_swap_keeps_newer_token() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, db) = temp_db();
        let store = make_store(&db, Arc::new(Mutex::new(None)));
        store
            .save(credentials("new-refresh"))
            .await
            .expect("保存失败");
        // 旧刷新失败不应清掉新 token
        store
            .on_refresh_token_rejected(&credentials("old-refresh"))
            .await
            .expect("rejected 处理失败");
        assert!(store.load().await.expect("读取失败").is_some());
        // 当前 refresh 被拒绝才清
        store
            .on_refresh_token_rejected(&credentials("new-refresh"))
            .await
            .expect("rejected 处理失败");
        assert!(store.load().await.expect("读取失败").is_none());
    }

    #[tokio::test]
    #[serial]
    async fn token_response_none_clears_cache() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
        let (_dir, db) = temp_db();
        let cache: OAuthTokenCache = Arc::new(Mutex::new(None));
        let store = make_store(&db, cache.clone());
        store
            .save(credentials("refresh-1"))
            .await
            .expect("保存失败");
        store
            .save(StoredCredentials::new(
                "client-1".to_string(),
                None,
                Vec::new(),
                None,
            ))
            .await
            .expect("保存空 token 失败");
        assert!(store.load().await.expect("读取失败").is_none());
        assert!(cache.lock().await.is_none());
    }
}
