//! OAuth 凭据仓储（v0.3.0）
//!
//! 一行对应一个服务器：
//! - 安装版：refresh token / client_secret 只存 keyring ref；
//! - 便携版：`refresh_token` / `client_secret` 明文；
//! - 非敏感元数据（client_id、scopes、issuer、过期时间、指纹）两种模式都存。

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

/// `client_secret` 来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSecretSource {
    User,
    Dcr,
}

impl ClientSecretSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientSecretSource::User => "user",
            ClientSecretSource::Dcr => "dcr",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(ClientSecretSource::User),
            "dcr" => Some(ClientSecretSource::Dcr),
            _ => None,
        }
    }
}

/// 实际使用的客户端注册方式（v0.3.1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationKind {
    Preregistered,
    Dcr,
    Cimd,
}

impl RegistrationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RegistrationKind::Preregistered => "preregistered",
            RegistrationKind::Dcr => "dcr",
            RegistrationKind::Cimd => "cimd",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "preregistered" => Some(RegistrationKind::Preregistered),
            "dcr" => Some(RegistrationKind::Dcr),
            "cimd" => Some(RegistrationKind::Cimd),
            _ => None,
        }
    }
}

/// OAuth 凭据行
#[derive(Debug, Clone)]
pub struct OAuthCredentialRow {
    pub server_id: String,
    pub refresh_ref: Option<String>,
    pub refresh_token: Option<String>,
    pub refresh_chunks: i64,
    pub client_id: Option<String>,
    pub client_secret_ref: Option<String>,
    pub client_secret: Option<String>,
    pub client_secret_source: Option<ClientSecretSource>,
    pub client_secret_version: i64,
    pub redirect_uri: Option<String>,
    pub scopes: Vec<String>,
    pub issuer: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub has_token: bool,
    pub has_refresh_token: bool,
    pub config_fingerprint: Option<String>,
    /// v0.3.1：注册方式；旧行可能为 None
    pub registration_kind: Option<RegistrationKind>,
    pub updated_at: DateTime<Utc>,
}

/// 读取单行
pub fn get(conn: &Connection, server_id: &str) -> anyhow::Result<Option<OAuthCredentialRow>> {
    let row = conn
        .query_row(
            "SELECT server_id, refresh_ref, refresh_token, refresh_chunks,
                    client_id, client_secret_ref, client_secret, client_secret_source,
                    client_secret_version, redirect_uri, scopes_json, issuer,
                    expires_at, has_token, has_refresh_token, config_fingerprint, updated_at,
                    registration_kind
             FROM oauth_credentials WHERE server_id = ?1",
            [server_id],
            map_row,
        )
        .optional()?;
    Ok(row)
}

/// 批量读取（列表页避免 N+1）
pub fn list_all(conn: &Connection) -> anyhow::Result<Vec<OAuthCredentialRow>> {
    let mut stmt = conn.prepare(
        "SELECT server_id, refresh_ref, refresh_token, refresh_chunks,
                client_id, client_secret_ref, client_secret, client_secret_source,
                client_secret_version, redirect_uri, scopes_json, issuer,
                expires_at, has_token, has_refresh_token, config_fingerprint, updated_at,
                registration_kind
         FROM oauth_credentials",
    )?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OAuthCredentialRow> {
    let scopes_json: String = row.get(10)?;
    let expires_at: Option<String> = row.get(12)?;
    let updated_at: String = row.get(16)?;
    let source: Option<String> = row.get(7)?;
    let registration_kind: Option<String> = row.get(17)?;
    Ok(OAuthCredentialRow {
        server_id: row.get(0)?,
        refresh_ref: row.get(1)?,
        refresh_token: row.get(2)?,
        refresh_chunks: row.get(3)?,
        client_id: row.get(4)?,
        client_secret_ref: row.get(5)?,
        client_secret: row.get(6)?,
        client_secret_source: source.as_deref().and_then(ClientSecretSource::parse),
        client_secret_version: row.get(8)?,
        redirect_uri: row.get(9)?,
        scopes: serde_json::from_str(&scopes_json).unwrap_or_default(),
        issuer: row.get(11)?,
        expires_at: expires_at.and_then(|text| text.parse().ok()),
        has_token: row.get::<_, i64>(13)? != 0,
        has_refresh_token: row.get::<_, i64>(14)? != 0,
        config_fingerprint: row.get(15)?,
        registration_kind: registration_kind
            .as_deref()
            .and_then(RegistrationKind::parse),
        updated_at: updated_at
            .parse()
            .unwrap_or_else(|_| DateTime::<Utc>::from_timestamp(0, 0).unwrap()),
    })
}

/// 写入/更新整行（调用方负责 keyring 与 DB 的一致性补偿）
pub fn upsert(conn: &Connection, row: &OAuthCredentialRow) -> anyhow::Result<()> {
    let scopes = serde_json::to_string(&row.scopes)?;
    conn.execute(
        "INSERT INTO oauth_credentials (
            server_id, refresh_ref, refresh_token, refresh_chunks,
            client_id, client_secret_ref, client_secret, client_secret_source,
            client_secret_version, redirect_uri, scopes_json, issuer,
            expires_at, has_token, has_refresh_token, config_fingerprint, updated_at,
            registration_kind
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
         )
         ON CONFLICT(server_id) DO UPDATE SET
            refresh_ref = excluded.refresh_ref,
            refresh_token = excluded.refresh_token,
            refresh_chunks = excluded.refresh_chunks,
            client_id = excluded.client_id,
            client_secret_ref = excluded.client_secret_ref,
            client_secret = excluded.client_secret,
            client_secret_source = excluded.client_secret_source,
            client_secret_version = excluded.client_secret_version,
            redirect_uri = excluded.redirect_uri,
            scopes_json = excluded.scopes_json,
            issuer = excluded.issuer,
            expires_at = excluded.expires_at,
            has_token = excluded.has_token,
            has_refresh_token = excluded.has_refresh_token,
            config_fingerprint = excluded.config_fingerprint,
            updated_at = excluded.updated_at,
            registration_kind = excluded.registration_kind",
        params![
            row.server_id,
            row.refresh_ref,
            row.refresh_token,
            row.refresh_chunks,
            row.client_id,
            row.client_secret_ref,
            row.client_secret,
            row.client_secret_source.map(ClientSecretSource::as_str),
            row.client_secret_version,
            row.redirect_uri,
            scopes,
            row.issuer,
            row.expires_at.map(|value| value.to_rfc3339()),
            i64::from(row.has_token),
            i64::from(row.has_refresh_token),
            row.config_fingerprint,
            row.updated_at.to_rfc3339(),
            row.registration_kind.map(RegistrationKind::as_str),
        ],
    )?;
    Ok(())
}

/// 删除整行
pub fn delete(conn: &Connection, server_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM oauth_credentials WHERE server_id = ?1",
        [server_id],
    )?;
    Ok(())
}

/// 只清理 token 字段，保留客户端注册
pub fn clear_token(conn: &Connection, server_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE oauth_credentials
         SET refresh_ref = NULL, refresh_token = NULL, refresh_chunks = 0,
             has_token = 0, has_refresh_token = 0, expires_at = NULL,
             updated_at = ?2
         WHERE server_id = ?1",
        params![server_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// 清 token + 客户端注册，保留用户配置（v0.3.1）
pub fn clear_registration(conn: &Connection, server_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE oauth_credentials
         SET refresh_ref = NULL, refresh_token = NULL, refresh_chunks = 0,
             client_id = NULL, client_secret_ref = NULL, client_secret = NULL,
             client_secret_source = NULL, registration_kind = NULL,
             redirect_uri = NULL, scopes_json = '[]', issuer = NULL,
             expires_at = NULL, has_token = 0, has_refresh_token = 0,
             config_fingerprint = NULL, updated_at = ?2
         WHERE server_id = ?1",
        params![server_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrations;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        migrations::run(&mut conn).expect("迁移失败");
        conn.execute(
            "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
             VALUES ('s1', 'demo', 'streamable_http', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("服务器插入失败");
        conn
    }

    fn row() -> OAuthCredentialRow {
        OAuthCredentialRow {
            server_id: "s1".to_string(),
            refresh_ref: Some("server/s1/oauth/refresh".to_string()),
            refresh_token: None,
            refresh_chunks: 2,
            client_id: Some("client-1".to_string()),
            client_secret_ref: None,
            client_secret: Some("secret".to_string()),
            client_secret_source: Some(ClientSecretSource::User),
            client_secret_version: 3,
            redirect_uri: Some("http://127.0.0.1:33318/oauth/callback".to_string()),
            scopes: vec!["a".to_string(), "b".to_string()],
            issuer: Some("https://issuer.example".to_string()),
            expires_at: Some(DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap()),
            has_token: true,
            has_refresh_token: true,
            config_fingerprint: Some("fp".to_string()),
            registration_kind: Some(RegistrationKind::Preregistered),
            updated_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(),
        }
    }

    #[test]
    fn upsert_get_list_clear_delete() {
        let conn = conn();
        upsert(&conn, &row()).expect("写入失败");
        let loaded = get(&conn, "s1").expect("查询失败").expect("行不存在");
        assert_eq!(loaded.client_id.as_deref(), Some("client-1"));
        assert_eq!(loaded.scopes, vec!["a", "b"]);
        assert_eq!(loaded.client_secret_source, Some(ClientSecretSource::User));
        assert_eq!(
            loaded.registration_kind,
            Some(RegistrationKind::Preregistered)
        );
        assert_eq!(loaded.refresh_chunks, 2);
        assert_eq!(list_all(&conn).expect("批量查询失败").len(), 1);

        clear_token(&conn, "s1").expect("清 token 失败");
        let cleared = get(&conn, "s1").expect("查询失败").unwrap();
        assert!(!cleared.has_token);
        assert!(!cleared.has_refresh_token);
        assert!(cleared.refresh_ref.is_none());
        assert_eq!(cleared.client_secret.as_deref(), Some("secret"));

        delete(&conn, "s1").expect("删除失败");
        assert!(get(&conn, "s1").expect("查询失败").is_none());
    }

    #[test]
    fn clear_registration_removes_identity_and_token() {
        let conn = conn();
        upsert(&conn, &row()).expect("写入失败");
        clear_registration(&conn, "s1").expect("清注册失败");
        let cleared = get(&conn, "s1").expect("查询失败").unwrap();
        assert!(cleared.client_id.is_none());
        assert!(cleared.registration_kind.is_none());
        assert!(cleared.refresh_ref.is_none());
        assert!(cleared.scopes.is_empty());
        assert!(cleared.issuer.is_none());
        assert!(!cleared.has_token);
    }

    #[test]
    fn refresh_and_secret_are_mutually_exclusive() {
        let conn = conn();
        let mut bad = row();
        bad.refresh_token = Some("plain".to_string());
        assert!(upsert(&conn, &bad).is_err(), "ref 与明文不能同时存在");

        let mut bad = row();
        bad.client_secret_ref = Some("ref".to_string());
        assert!(
            upsert(&conn, &bad).is_err(),
            "secret ref 与明文不能同时存在"
        );
    }
}
