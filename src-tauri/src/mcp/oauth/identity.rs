//! OAuth 客户端注册身份解析（v0.3.1）
//!
//! 原则：
//! - `StoredIdentity` 只读、无网络、无副作用，供连接/状态/诊断使用；
//! - `FreshLoginIdentity` 只在 `begin_login` 的 metadata discovery 之后
//!   使用，可用 CIMD support 与本次实际回调地址；
//! - 清理旧 token/secret 只由 service 层的 `reconcile_identity` 执行，
//!   身份解析函数本身不写数据库。

use rmcp::transport::auth::AuthorizationMetadata;

use crate::mcp::models::OAuthConfig;
use crate::storage::oauth_repo::{ClientSecretSource, OAuthCredentialRow, RegistrationKind};

use super::error::OAuthError;

/// 已持久化的客户端身份（只读）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIdentity {
    pub kind: Option<RegistrationKind>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub has_secret: bool,
}

impl StoredIdentity {
    /// 配置保存后的目标身份（只读，不联网）。
    ///
    /// 规则：显式 client_id 优先；已有 DCR 会话不因新增 CIMD 配置而切换；
    /// 已有 CIMD 会话在 CIMD 配置变更时跟随新 URL；移除 CIMD/预注册配置
    /// 则清空身份。
    pub fn for_config(oauth: &OAuthConfig, row: Option<&OAuthCredentialRow>) -> Self {
        if oauth.client_id.is_some() {
            return Self::resolve(oauth, row);
        }
        if let Some(row) = row {
            match row.registration_kind {
                Some(RegistrationKind::Dcr) => {
                    return Self {
                        kind: Some(RegistrationKind::Dcr),
                        client_id: row.client_id.clone(),
                        redirect_uri: row.redirect_uri.clone(),
                        has_secret: has_secret(row),
                    };
                }
                Some(RegistrationKind::Cimd) => {
                    return match oauth.client_metadata_url.clone() {
                        Some(url) => Self {
                            kind: Some(RegistrationKind::Cimd),
                            client_id: Some(url),
                            redirect_uri: row.redirect_uri.clone(),
                            has_secret: false,
                        },
                        None => Self {
                            kind: None,
                            client_id: None,
                            redirect_uri: row.redirect_uri.clone(),
                            has_secret: false,
                        },
                    };
                }
                Some(RegistrationKind::Preregistered) => {
                    return Self {
                        kind: None,
                        client_id: None,
                        redirect_uri: row.redirect_uri.clone(),
                        has_secret: false,
                    };
                }
                None => {
                    if row.client_secret_source == Some(ClientSecretSource::Dcr) {
                        return Self {
                            kind: Some(RegistrationKind::Dcr),
                            client_id: row.client_id.clone(),
                            redirect_uri: row.redirect_uri.clone(),
                            has_secret: has_secret(row),
                        };
                    }
                }
            }
        }
        match oauth.client_metadata_url.clone() {
            Some(url) => Self {
                kind: Some(RegistrationKind::Cimd),
                client_id: Some(url),
                redirect_uri: row.and_then(|row| row.redirect_uri.clone()),
                has_secret: false,
            },
            None => Self {
                kind: None,
                client_id: None,
                redirect_uri: row.and_then(|row| row.redirect_uri.clone()),
                has_secret: false,
            },
        }
    }

    /// 只读解析：config client_id > row registration_kind > legacy source。
    pub fn resolve(oauth: &OAuthConfig, row: Option<&OAuthCredentialRow>) -> Self {
        if let Some(client_id) = oauth.client_id.clone() {
            return Self {
                kind: Some(RegistrationKind::Preregistered),
                client_id: Some(client_id),
                redirect_uri: row.and_then(|row| row.redirect_uri.clone()),
                has_secret: row.is_some_and(has_secret),
            };
        }

        if let Some(row) = row {
            let kind = row.registration_kind.or_else(|| {
                match row.client_secret_source {
                    Some(ClientSecretSource::Dcr) => Some(RegistrationKind::Dcr),
                    Some(ClientSecretSource::User) => Some(RegistrationKind::Preregistered),
                    // v0.3.0 的预注册公共客户端：source NULL、有 client_id、有 token
                    None if row.client_id.is_some() && row.has_token => {
                        Some(RegistrationKind::Preregistered)
                    }
                    None => None,
                }
            });
            return Self {
                kind,
                client_id: row.client_id.clone(),
                redirect_uri: row.redirect_uri.clone(),
                has_secret: has_secret(row),
            };
        }

        Self {
            kind: None,
            client_id: None,
            redirect_uri: None,
            has_secret: false,
        }
    }
}

/// 本次登录将使用的客户端身份
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshLoginIdentity {
    pub kind: RegistrationKind,
    pub client_id: Option<String>,
    pub redirect_uri: String,
    pub needs_registration: bool,
}

impl FreshLoginIdentity {
    /// 只在 `begin_login` 的 metadata discovery 之后调用。
    pub fn resolve(
        oauth: &OAuthConfig,
        row: Option<&OAuthCredentialRow>,
        metadata: &AuthorizationMetadata,
        actual_redirect: &str,
    ) -> Result<Self, OAuthError> {
        let cimd_supported = cimd_supported(metadata);
        let stored = StoredIdentity::resolve(oauth, row);

        // 1. 显式预注册永远优先
        if let Some(client_id) = oauth.client_id.clone() {
            return Ok(Self {
                kind: RegistrationKind::Preregistered,
                client_id: Some(client_id),
                redirect_uri: actual_redirect.to_string(),
                needs_registration: false,
            });
        }

        // 2. 复用持久化身份
        if let Some(kind) = stored.kind {
            match kind {
                RegistrationKind::Dcr => {
                    let reusable = stored.client_id.is_some()
                        && stored.redirect_uri.as_deref() == Some(actual_redirect);
                    return Ok(Self {
                        kind: RegistrationKind::Dcr,
                        client_id: if reusable {
                            stored.client_id.clone()
                        } else {
                            None
                        },
                        redirect_uri: actual_redirect.to_string(),
                        needs_registration: !reusable,
                    });
                }
                RegistrationKind::Cimd => {
                    if let Some(url) = oauth.client_metadata_url.clone()
                        && stored.client_id.as_deref() == Some(url.as_str())
                        && cimd_supported
                    {
                        return Ok(Self {
                            kind: RegistrationKind::Cimd,
                            client_id: Some(url),
                            redirect_uri: actual_redirect.to_string(),
                            needs_registration: false,
                        });
                    }
                    // Cimd 行已不可用：有 DCR endpoint 则项目自己重新注册，
                    // 否则明确报错，绝不交给 rmcp 内部 DCR。
                    return dcr_or_error(metadata, actual_redirect);
                }
                RegistrationKind::Preregistered => {
                    // config client_id 已在上面的分支处理；走到这里说明
                    // 配置已移除，按未注册处理。
                }
            }
        }

        // 3. 没有可用持久化身份
        if let Some(url) = oauth.client_metadata_url.clone()
            && cimd_supported
        {
            return Ok(Self {
                kind: RegistrationKind::Cimd,
                client_id: Some(url),
                redirect_uri: actual_redirect.to_string(),
                needs_registration: false,
            });
        }
        dcr_or_error(metadata, actual_redirect)
    }
}

fn dcr_or_error(
    metadata: &AuthorizationMetadata,
    actual_redirect: &str,
) -> Result<FreshLoginIdentity, OAuthError> {
    if metadata.registration_endpoint.is_some() {
        Ok(FreshLoginIdentity {
            kind: RegistrationKind::Dcr,
            client_id: None,
            redirect_uri: actual_redirect.to_string(),
            needs_registration: true,
        })
    } else {
        Err(OAuthError::Registration {
            message: "授权服务器不支持 CIMD，也没有 registration_endpoint；\
                      请填写 client_id 或配置 CIMD"
                .to_string(),
        })
    }
}

fn has_secret(row: &OAuthCredentialRow) -> bool {
    row.client_secret.is_some() || row.client_secret_ref.is_some()
}

/// AS 是否广告 CIMD（SEP-991）
pub fn cimd_supported(metadata: &AuthorizationMetadata) -> bool {
    metadata
        .additional_fields
        .get("client_id_metadata_document_supported")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn oauth(client_id: Option<&str>, cimd: Option<&str>) -> OAuthConfig {
        OAuthConfig {
            client_id: client_id.map(ToString::to_string),
            client_metadata_url: cimd.map(ToString::to_string),
            ..Default::default()
        }
    }

    fn row(kind: Option<RegistrationKind>, client_id: &str) -> OAuthCredentialRow {
        OAuthCredentialRow {
            server_id: "s1".to_string(),
            refresh_ref: None,
            refresh_token: None,
            refresh_chunks: 0,
            client_id: Some(client_id.to_string()),
            client_secret_ref: None,
            client_secret: None,
            client_secret_source: None,
            client_secret_version: 0,
            redirect_uri: Some("http://127.0.0.1:33318/oauth/callback".to_string()),
            scopes: Vec::new(),
            issuer: None,
            expires_at: None,
            has_token: true,
            has_refresh_token: true,
            config_fingerprint: None,
            registration_kind: kind,
            updated_at: Utc::now(),
        }
    }

    fn metadata(cimd: bool, dcr: bool) -> AuthorizationMetadata {
        let mut metadata = AuthorizationMetadata::default();
        if dcr {
            metadata.registration_endpoint = Some("https://auth.example.com/register".to_string());
        }
        metadata.additional_fields.insert(
            "client_id_metadata_document_supported".to_string(),
            serde_json::Value::Bool(cimd),
        );
        metadata
    }

    #[test]
    fn preregistered_wins() {
        let identity = FreshLoginIdentity::resolve(
            &oauth(Some("client-1"), Some("https://client.example.com/cimd.json")),
            Some(&row(Some(RegistrationKind::Dcr), "dcr-1")),
            &metadata(true, true),
            "http://127.0.0.1:33318/oauth/callback",
        )
        .expect("resolve 失败");
        assert_eq!(identity.kind, RegistrationKind::Preregistered);
        assert_eq!(identity.client_id.as_deref(), Some("client-1"));
    }

    #[test]
    fn dcr_row_survives_new_cimd_config() {
        let identity = FreshLoginIdentity::resolve(
            &oauth(None, Some("https://client.example.com/cimd.json")),
            Some(&row(Some(RegistrationKind::Dcr), "dcr-1")),
            &metadata(true, true),
            "http://127.0.0.1:33318/oauth/callback",
        )
        .expect("resolve 失败");
        assert_eq!(identity.kind, RegistrationKind::Dcr);
        assert_eq!(identity.client_id.as_deref(), Some("dcr-1"));
        assert!(!identity.needs_registration);
    }

    #[test]
    fn cimd_row_without_support_falls_back_to_project_dcr() {
        let identity = FreshLoginIdentity::resolve(
            &oauth(None, Some("https://client.example.com/cimd.json")),
            Some(&row(Some(RegistrationKind::Cimd), "https://client.example.com/cimd.json")),
            &metadata(false, true),
            "http://127.0.0.1:33318/oauth/callback",
        )
        .expect("resolve 失败");
        assert_eq!(identity.kind, RegistrationKind::Dcr);
        assert!(identity.needs_registration);
    }

    #[test]
    fn cimd_url_change_updates_identity() {
        let target = StoredIdentity::for_config(
            &oauth(None, Some("https://client.example.com/new.json")),
            Some(&row(
                Some(RegistrationKind::Cimd),
                "https://client.example.com/old.json",
            )),
        );
        assert_eq!(target.kind, Some(RegistrationKind::Cimd));
        assert_eq!(
            target.client_id.as_deref(),
            Some("https://client.example.com/new.json")
        );
    }

    #[test]
    fn removing_cimd_clears_identity() {
        let target = StoredIdentity::for_config(
            &oauth(None, None),
            Some(&row(
                Some(RegistrationKind::Cimd),
                "https://client.example.com/cimd.json",
            )),
        );
        assert_eq!(target.kind, None);
        assert_eq!(target.client_id, None);
    }

    #[test]
    fn legacy_null_source_prefers_preregistered() {
        let mut row = row(None, "legacy-client");
        row.client_secret_source = None;
        let identity = StoredIdentity::resolve(&oauth(None, None), Some(&row));
        assert_eq!(identity.kind, Some(RegistrationKind::Preregistered));
    }
}
