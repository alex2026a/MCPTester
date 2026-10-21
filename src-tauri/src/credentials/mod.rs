//! 凭据存储模块（敏感 HTTP 头 / env 值）
//!
//! 基于 keyring-core 1.0 + 平台原生 store（keyring v4 生态，官方推荐应用侧直接使用）：
//! - Windows：Windows Credential Manager（windows-native-keyring-store）
//! - macOS：Keychain Services（apple-native-keyring-store）
//! - Linux：Secret Service（zbus-secret-service-keyring-store）
//!
//! 设计约束：
//! - 键名版本化：`server/{server_id}/env/{key}/{nonce}`、`server/{server_id}/header/{name}/{nonce}`，
//!   同名键更新 = 写新 nonce 后切引用（由 credentials 表记录权威 ref），旧条目随后删除；
//! - 不依赖 keyring 枚举 API（各平台后端能力不一致），孤儿判定以本地索引为准；
//! - 敏感值不落日志（由 logging 模块脱敏保证）；
//! - 单测使用 keyring-core mock store（全局共享，需 serial_test 串行）。

// M1 脚手架：凭据读写接口在 M2 服务器管理接入后被调用，届时移除此 allow
#![allow(dead_code)]

use keyring_core::api::CredentialPersistence;

use crate::core::errors::AppError;

/// keyring 服务名
pub const SERVICE: &str = "mcptester";

/// 敏感值类别（与 04 文档 credentials.kind 对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    /// stdio 服务器 env 变量
    Env,
    /// HTTP 服务器请求头
    Header,
}

impl SecretKind {
    /// 键名路径片段
    pub fn path_segment(self) -> &'static str {
        match self {
            SecretKind::Env => "env",
            SecretKind::Header => "header",
        }
    }
}

/// 凭据持久化状态（启动自检用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceStatus {
    /// 默认凭据库未设置（初始化失败）
    NoStore,
    /// 凭据库不具备持久化能力（如 mock / 内存存储），重启后丢失
    NonPersistent,
    /// 凭据库可持久化
    Persistent,
}

/// 初始化平台凭据库（必须在任何凭据操作前调用一次）
pub fn init_store() -> Result<(), AppError> {
    let store = build_platform_store()?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "windows")]
fn build_platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, AppError> {
    let store = windows_native_keyring_store::Store::new()
        .map_err(|e| AppError::CredentialStoreUnavailable(e.to_string()))?;
    let store: std::sync::Arc<keyring_core::CredentialStore> = store;
    Ok(store)
}

#[cfg(target_os = "macos")]
fn build_platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, AppError> {
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|e| AppError::CredentialStoreUnavailable(e.to_string()))?;
    let store: std::sync::Arc<keyring_core::CredentialStore> = store;
    Ok(store)
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn build_platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, AppError> {
    let store = zbus_secret_service_keyring_store::Store::new()
        .map_err(|e| AppError::CredentialStoreUnavailable(e.to_string()))?;
    let store: std::sync::Arc<keyring_core::CredentialStore> = store;
    Ok(store)
}

/// 凭据库持久化能力自检（见 05 实施计划 M1 任务 5）
pub fn persistence_status() -> PersistenceStatus {
    match keyring_core::get_default_store() {
        None => PersistenceStatus::NoStore,
        Some(store) => match store.persistence() {
            // 凭据落盘（应用重启、注销、重启后仍存在）
            CredentialPersistence::UntilLogout
            | CredentialPersistence::UntilReboot
            | CredentialPersistence::UntilDelete => PersistenceStatus::Persistent,
            // EntryOnly / ProcessOnly / Unspecified 等视为不具备持久化能力
            _ => PersistenceStatus::NonPersistent,
        },
    }
}

/// 生成版本化键名的 nonce（UUIDv4）
pub fn new_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 构造版本化键名：`server/{server_id}/{env|header}/{name}/{nonce}`
pub fn secret_ref(server_id: &str, kind: SecretKind, name: &str, nonce: &str) -> String {
    format!(
        "server/{server_id}/{}/{name}/{nonce}",
        kind.path_segment()
    )
}

/// 写入敏感值（新增；键名由调用方按版本化规则生成）
pub fn set_secret(reference: &str, secret: &str) -> Result<(), AppError> {
    let entry = keyring_core::Entry::new(SERVICE, reference).map_err(map_store_error)?;
    entry.set_password(secret).map_err(map_store_error)
}

/// 读取敏感值
pub fn get_secret(reference: &str) -> Result<String, AppError> {
    let entry = keyring_core::Entry::new(SERVICE, reference).map_err(map_store_error)?;
    entry.get_password().map_err(map_store_error)
}

/// 删除敏感值；条目不存在视为成功（幂等）
pub fn delete_secret(reference: &str) -> Result<(), AppError> {
    let entry = keyring_core::Entry::new(SERVICE, reference).map_err(map_store_error)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(map_store_error(e)),
    }
}

/// 错误映射：NoEntry 归一为 CredentialNotFound
fn map_store_error(e: keyring_core::Error) -> AppError {
    match e {
        keyring_core::Error::NoEntry => AppError::CredentialNotFound,
        keyring_core::Error::NoDefaultStore => {
            AppError::CredentialStoreUnavailable("默认凭据库未初始化".to_string())
        }
        other => AppError::Credential(other.to_string()),
    }
}

/// 敏感值掩码展示（前端仅可获得掩码）：`Bear****abcd` / `sk-****5678`
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() < 8 {
        return "****".to_string();
    }
    let prefix_len = if secret.starts_with("Bearer ") {
        5
    } else if secret.starts_with("sk-") {
        3
    } else {
        4
    };
    let prefix: String = chars.iter().take(prefix_len).collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}****{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// 将默认凭据库替换为 mock store（全局共享，测试须串行）
    fn use_mock_store() {
        keyring_core::unset_default_store();
        keyring_core::set_default_store(
            keyring_core::mock::Store::new().expect("mock store 创建失败"),
        );
    }

    #[test]
    #[serial]
    fn secret_roundtrip() {
        // 敏感值写入、读取、删除全流程（版本化键名）
        use_mock_store();
        let reference = secret_ref("srv-1", SecretKind::Header, "Authorization", "nonce-1");

        set_secret(&reference, "Bearer abcdefgh12345678").expect("写入失败");
        assert_eq!(
            get_secret(&reference).expect("读取失败"),
            "Bearer abcdefgh12345678"
        );

        delete_secret(&reference).expect("删除失败");
        assert!(matches!(
            get_secret(&reference),
            Err(AppError::CredentialNotFound)
        ));
    }

    #[test]
    #[serial]
    fn delete_missing_is_ok() {
        // 删除不存在的敏感值视为成功（幂等）
        use_mock_store();
        assert!(delete_secret("no/such/ref").is_ok());
    }

    #[test]
    #[serial]
    fn get_missing_returns_not_found() {
        use_mock_store();
        assert!(matches!(
            get_secret("no/such/ref"),
            Err(AppError::CredentialNotFound)
        ));
    }

    #[test]
    #[serial]
    fn mock_store_reports_non_persistent() {
        use_mock_store();
        assert_eq!(persistence_status(), PersistenceStatus::NonPersistent);
    }

    #[test]
    #[serial]
    fn no_store_status() {
        // 未初始化存储时状态为 NoStore（须串行：会清空全局默认存储）
        keyring_core::unset_default_store();
        assert_eq!(persistence_status(), PersistenceStatus::NoStore);
    }

    #[test]
    fn key_name_format() {
        // 版本化键名格式：server/{id}/{env|header}/{name}/{nonce}
        assert_eq!(
            secret_ref("abc-123", SecretKind::Env, "GITHUB_TOKEN", "n1"),
            "server/abc-123/env/GITHUB_TOKEN/n1"
        );
        assert_eq!(
            secret_ref("abc-123", SecretKind::Header, "Authorization", "n2"),
            "server/abc-123/header/Authorization/n2"
        );
    }

    #[test]
    fn mask_rules() {
        // 掩码规则（前端展示）
        assert_eq!(mask("Bearer abcdefgh12345678"), "Beare****5678");
        assert_eq!(mask("sk-abcdefgh12345678"), "sk-****5678");
        assert_eq!(mask("abcd1234efgh"), "abcd****efgh");
        assert_eq!(mask("short"), "****");
    }
}
