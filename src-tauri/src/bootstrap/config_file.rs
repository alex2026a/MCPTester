//! 启动配置文件（`mcptester.config.json`）发现、解析与一次性 seed。
//!
//! 语义：只补充尚未应用过的 key，永不覆盖已有服务器；用户删除预置服务器后
//! 不会自动复活（`app_meta.bootstrap.applied` 记录）。设置页可手动"重新应用"
//! 补回缺失项。

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::bootstrap::state as bootstrap_state;
use crate::commands::AppState;
use crate::commands::import_export::{SecretHints, build_draft_with_options, detect_entries};
use crate::commands::server::{
    apply_plaintext_secrets, draft_to_config, load_resolved_config, validate_draft,
};
use crate::mcp::models::ServerConfig;
use crate::mcp::models::ServerOrigin;
use crate::storage::server_repo;

/// 启动配置文件名
pub const CONFIG_FILE: &str = "mcptester.config.json";

/// 单个配置条目的失败原因
#[derive(Debug, Clone, Serialize)]
pub struct SeedIssue {
    pub key: String,
    pub reason: String,
}

/// seed / 重新应用结果（前端展示）
#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    /// not_found / applied / partial / error
    pub status: String,
    pub path: Option<String>,
    pub added: u32,
    pub skipped: u32,
    pub failed: Vec<SeedIssue>,
    pub default_server_id: Option<String>,
    pub auto_connect: bool,
}

/// 配置状态（AppInfo 使用）
#[derive(Debug, Clone)]
pub struct ConfigStatus {
    pub path: Option<String>,
    /// not_found / pending / applied / partial / error
    pub status: String,
    pub error: Option<String>,
    pub changed: bool,
    pub default_server_id: Option<String>,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOptions {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    auto_connect: bool,
    #[serde(default)]
    default_server: Option<String>,
    #[serde(default)]
    secret_headers: Vec<String>,
    #[serde(default)]
    secret_env: Vec<String>,
}

impl Default for ConfigOptions {
    fn default() -> Self {
        Self {
            version: 1,
            auto_connect: false,
            default_server: None,
            secret_headers: Vec::new(),
            secret_env: Vec::new(),
        }
    }
}

fn default_version() -> u32 {
    1
}

struct ConfigDocument {
    options: ConfigOptions,
    entries: Vec<(String, Value)>,
}

/// 启动时自动 seed（只补缺失项）
pub fn seed(state: &AppState, resource_dir: Option<&Path>, include_applied: bool) -> SeedReport {
    let Some(path) = discover(state, resource_dir) else {
        return SeedReport {
            status: "not_found".to_string(),
            path: None,
            added: 0,
            skipped: 0,
            failed: Vec::new(),
            default_server_id: None,
            auto_connect: false,
        };
    };
    seed_file(state, &path, include_applied)
}

/// 对指定文件执行 seed（便于单测）
pub(crate) fn seed_file(state: &AppState, path: &Path, include_applied: bool) -> SeedReport {
    let path_text = path.display().to_string();
    let text = match read_config_text(path) {
        Ok(text) => text,
        Err(reason) => {
            save_error(state, Some(&path_text), reason.clone());
            return SeedReport {
                status: "error".to_string(),
                path: Some(path_text),
                added: 0,
                skipped: 0,
                failed: vec![SeedIssue {
                    key: "__config__".to_string(),
                    reason,
                }],
                default_server_id: None,
                auto_connect: false,
            };
        }
    };
    let hash = hash_text(&text);

    let document = match parse_config(&text) {
        Ok(document) => document,
        Err(reason) => {
            save_error(state, Some(&path_text), reason.clone());
            return SeedReport {
                status: "error".to_string(),
                path: Some(path_text),
                added: 0,
                skipped: 0,
                failed: vec![SeedIssue {
                    key: "__config__".to_string(),
                    reason,
                }],
                default_server_id: None,
                auto_connect: false,
            };
        }
    };

    let state_before = state
        .db
        .with_read_blocking(bootstrap_state::load)
        .unwrap_or_default();
    let first_seed = state_before.applied.is_empty();
    let hints = SecretHints {
        headers: document.options.secret_headers.clone(),
        env: document.options.secret_env.clone(),
    };

    let mut prepared: Vec<(String, String, ServerConfig)> = Vec::new();
    let mut failed: Vec<SeedIssue> = Vec::new();
    let mut skipped = 0u32;

    for (key, value) in &document.entries {
        if !include_applied && state_before.applied.contains_key(key) {
            skipped += 1;
            continue;
        }
        let draft = match build_draft_with_options(key, value, true, &hints) {
            Ok(draft) => draft,
            Err(reason) => {
                failed.push(SeedIssue {
                    key: key.clone(),
                    reason,
                });
                continue;
            }
        };
        if let Err(reason) = validate_draft(&draft) {
            failed.push(SeedIssue {
                key: key.clone(),
                reason,
            });
            continue;
        }

        let id = seed_server_id(key);
        let now = Utc::now();
        let mut config = draft_to_config(&draft, id.clone(), now, now, false);
        apply_plaintext_secrets(&mut config, &draft.secrets, None);
        config.origin = Some(ServerOrigin {
            source: "config".to_string(),
            key: key.clone(),
        });
        prepared.push((key.clone(), id, config));
    }

    let error_summary = if failed.is_empty() {
        None
    } else {
        Some(
            failed
                .iter()
                .map(|issue| format!("{}：{}", issue.key, issue.reason))
                .collect::<Vec<_>>()
                .join("；"),
        )
    };
    let options = document.options.clone();
    let current_keys: Vec<String> = document
        .entries
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    let path_for_tx = path_text.clone();
    let hash_for_tx = hash.clone();
    let mut new_state = state_before.clone();

    let result = state.db.with_write_blocking(|conn| {
        let tx = conn.transaction()?;
        let mut added = 0u32;
        let mut skipped_in_tx = skipped;

        for (key, id, config) in &prepared {
            if server_repo::get(&tx, id)?.is_some() {
                new_state.applied.insert(key.clone(), id.clone());
                skipped_in_tx += 1;
                continue;
            }
            server_repo::upsert(&tx, config)?;
            new_state.applied.insert(key.clone(), id.clone());
            added += 1;
        }

        // 首次 seed 或手动重新应用时，同步默认服务器/自动连接设置
        if first_seed || include_applied {
            new_state.default_server_id = options
                .default_server
                .as_ref()
                .and_then(|key| new_state.applied.get(key).cloned())
                .or_else(|| {
                    current_keys
                        .iter()
                        .find_map(|key| new_state.applied.get(key).cloned())
                });
            new_state.auto_connect = options.auto_connect;
        }

        new_state.path = Some(path_for_tx.clone());
        // hash 表示“最后一次完整应用的文件版本”；后续自动启动即使补了新 key，
        // 也不更新，以便设置页继续提示配置文件有变化。
        if first_seed || include_applied {
            new_state.hash = Some(hash_for_tx.clone());
        }
        if first_seed || include_applied || added > 0 {
            new_state.applied_at = Some(Utc::now());
        }
        new_state.last_status = if error_summary.is_none() {
            "applied".to_string()
        } else {
            "partial".to_string()
        };
        new_state.last_error = error_summary.clone();
        bootstrap_state::save(&tx, &new_state)?;
        tx.commit()?;
        Ok((added, skipped_in_tx))
    });

    match result {
        Ok((added, skipped)) => SeedReport {
            status: if failed.is_empty() {
                "applied".to_string()
            } else {
                "partial".to_string()
            },
            path: Some(path_text),
            added,
            skipped,
            failed,
            default_server_id: new_state.default_server_id,
            auto_connect: new_state.auto_connect,
        },
        Err(error) => {
            let mut all_failed = failed;
            all_failed.push(SeedIssue {
                key: "__db__".to_string(),
                reason: format!("写入数据库失败：{error}"),
            });
            SeedReport {
                status: "error".to_string(),
                path: Some(path_text),
                added: 0,
                skipped,
                failed: all_failed,
                default_server_id: new_state.default_server_id,
                auto_connect: new_state.auto_connect,
            }
        }
    }
}

/// 配置状态（不修改数据库）
pub fn status(state: &AppState, resource_dir: Option<&Path>) -> ConfigStatus {
    let state_before = state
        .db
        .with_read_blocking(bootstrap_state::load)
        .unwrap_or_default();
    let Some(path) = discover(state, resource_dir) else {
        return ConfigStatus {
            path: None,
            status: "not_found".to_string(),
            error: state_before.last_error,
            changed: false,
            default_server_id: state_before.default_server_id,
            auto_connect: state_before.auto_connect,
        };
    };

    let current_hash = read_config_text(&path).ok().map(|text| hash_text(&text));
    let changed = current_hash.is_some() && current_hash != state_before.hash;
    let status = if state_before.last_status.is_empty() {
        "pending".to_string()
    } else {
        state_before.last_status
    };
    ConfigStatus {
        path: Some(path.display().to_string()),
        status,
        error: state_before.last_error,
        changed,
        default_server_id: state_before.default_server_id,
        auto_connect: state_before.auto_connect,
    }
}

/// 发现配置文件：--config > MCPTESTER_CONFIG > exe 同级 > 数据目录 > resource_dir
pub fn discover(state: &AppState, resource_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = cli_config_path() {
        candidates.push(path);
    }
    if let Some(path) = std::env::var_os("MCPTESTER_CONFIG") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(exe_dir) = exe_dir() {
        candidates.push(exe_dir.join(CONFIG_FILE));
    }
    candidates.push(state.data_dir.join(CONFIG_FILE));
    if let Some(dir) = resource_dir {
        candidates.push(dir.join(CONFIG_FILE));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// 启动后后台连接默认服务器（不阻塞窗口）
pub fn spawn_auto_connect(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // 给前端首帧和事件转发一点启动时间
        tokio::time::sleep(Duration::from_millis(500)).await;
        let state = app.state::<AppState>();
        let bootstrap_state = match state.db.with_read_blocking(bootstrap_state::load) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(error = %error, "读取启动配置状态失败，跳过自动连接");
                return;
            }
        };
        if !bootstrap_state.auto_connect {
            return;
        }

        let servers = state
            .db
            .with_read_blocking(server_repo::list)
            .unwrap_or_default();
        let target = bootstrap_state
            .default_server_id
            .filter(|id| {
                servers
                    .iter()
                    .any(|server| &server.id == id && server.enabled)
            })
            .or_else(|| {
                servers
                    .iter()
                    .find(|server| server.enabled && server.origin.is_some())
                    .map(|server| server.id.clone())
            });
        let Some(id) = target else {
            tracing::info!("启动配置启用自动连接，但没有可用的默认服务器");
            return;
        };

        let config = match load_resolved_config(state.inner(), &id) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(server_id = %id, error = %error.message, "自动连接前解析配置失败");
                return;
            }
        };
        if !config.enabled {
            return;
        }
        if let Some(handle) = state.manager.handle(&id).await
            && handle.state().await.is_ready()
        {
            return;
        }
        tracing::info!(server_id = %id, name = %config.name, "自动连接默认服务器");
        match state.manager.connect(&config).await {
            Ok(_) => {
                let _ = state.db.with_write_blocking(|conn| {
                    server_repo::set_last_connected(conn, &id, Utc::now())
                });
            }
            Err(error) => {
                tracing::warn!(
                    server_id = %id,
                    error = %error.message(),
                    "自动连接默认服务器失败"
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

fn parse_config(text: &str) -> Result<ConfigDocument, String> {
    let root: Value = serde_json::from_str(text).map_err(|error| {
        format!(
            "JSON 解析失败（第 {} 行第 {} 列）：{error}",
            error.line(),
            error.column()
        )
    })?;
    let options = match root.get("mcptester") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("mcptester 配置无效：{error}"))?,
        None => ConfigOptions::default(),
    };
    if options.version != 1 {
        return Err(format!(
            "不支持的配置版本：{}（当前仅支持 1）",
            options.version
        ));
    }
    let entries = detect_entries(&root)
        .ok_or_else(|| "无法识别格式（需为 servers 或 mcpServers 结构）".to_string())?;
    Ok(ConfigDocument { options, entries })
}

fn read_config_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("读取配置文件失败「{}」：{error}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("配置文件不是有效 UTF-8「{}」：{error}", path.display()))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

fn save_error(state: &AppState, path: Option<&str>, reason: String) {
    let mut current = state
        .db
        .with_read_blocking(bootstrap_state::load)
        .unwrap_or_default();
    current.path = path.map(ToString::to_string).or(current.path);
    current.last_status = "error".to_string();
    current.last_error = Some(reason);
    let _ = state
        .db
        .with_write_blocking(|conn| bootstrap_state::save(conn, &current));
}

fn seed_server_id(key: &str) -> String {
    // 确定性 ID：重复 seed / 重新应用时命中同一行，避免按名称匹配
    format!("cfg:{key}")
}

fn cli_config_path() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            return args.next().map(PathBuf::from);
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

/// FNV-1a 64：仅用于文件变化检测，不作为安全哈希
fn hash_text(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ConnectionManager;
    use crate::mcp::models::{RuntimeSettings, TransportKind};
    use crate::storage::Database;
    use std::sync::Arc;

    fn test_state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let db = Database::open(&dir.path().join("test.db")).expect("数据库打开失败");
        db.migrate().expect("迁移失败");
        let state = AppState {
            db,
            manager: Arc::new(ConnectionManager::new(RuntimeSettings::default())),
            data_dir: dir.path().to_path_buf(),
            log_dir: dir.path().join("logs"),
            portable: true,
            data_dir_source: "portable".to_string(),
        };
        (dir, state)
    }

    const CONFIG: &str = r#"{
        "mcptester": {
            "version": 1,
            "autoConnect": true,
            "defaultServer": "remote"
        },
        "servers": {
            "remote": {
                "name": "远程",
                "type": "http",
                "url": "http://127.0.0.1:8080/mcp",
                "headers": { "Authorization": "Bearer abc123456" }
            },
            "fs": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "pkg", "D:\\data"]
            }
        }
    }"#;

    #[test]
    fn seed_creates_plaintext_servers_and_records_state() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, CONFIG).expect("写配置失败");

        let report = seed_file(&state, &path, false);
        assert_eq!(report.status, "applied");
        assert_eq!(report.added, 2);

        let servers = state
            .db
            .with_read_blocking(server_repo::list)
            .expect("读取服务器失败");
        assert_eq!(servers.len(), 2);
        let remote = servers
            .iter()
            .find(|server| {
                server.origin.as_ref().map(|origin| origin.key.as_str()) == Some("remote")
            })
            .expect("缺少 remote");
        assert_eq!(remote.name, "远程");
        assert_eq!(remote.transport, TransportKind::StreamableHttp);
        let auth = &remote.http.as_ref().unwrap().headers[0];
        assert_eq!(auth.key, "Authorization");
        assert_eq!(auth.value.as_deref(), Some("Bearer abc123456"));
        assert!(auth.secret);

        // 明文路径不写凭据索引
        let refs = state
            .db
            .with_read_blocking(server_repo::list_all_credentials)
            .expect("读取凭据索引失败");
        assert!(refs.is_empty());

        let bootstrap = state
            .db
            .with_read_blocking(bootstrap_state::load)
            .expect("读取状态失败");
        assert_eq!(bootstrap.default_server_id.as_deref(), Some("cfg:remote"));
        assert!(bootstrap.auto_connect);
        assert_eq!(bootstrap.applied.len(), 2);
    }

    #[test]
    fn seed_is_idempotent_and_deleted_server_does_not_resurrect() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, CONFIG).expect("写配置失败");

        seed_file(&state, &path, false);
        let second = seed_file(&state, &path, false);
        assert_eq!(second.added, 0);
        assert_eq!(second.skipped, 2);

        // 删除 remote 后再自动 seed：不应复活
        state
            .db
            .with_write_blocking(|conn| server_repo::delete(conn, "cfg:remote"))
            .expect("删除失败");
        let third = seed_file(&state, &path, false);
        assert_eq!(third.added, 0);
        assert!(
            state
                .db
                .with_read_blocking(|conn| server_repo::get(conn, "cfg:remote"))
                .expect("查询失败")
                .is_none()
        );

        // 手动重新应用：补回缺失项，已有项不覆盖
        let fourth = seed_file(&state, &path, true);
        assert_eq!(fourth.added, 1);
        assert!(
            state
                .db
                .with_read_blocking(|conn| server_repo::get(conn, "cfg:remote"))
                .expect("查询失败")
                .is_some()
        );
    }

    #[test]
    fn seed_does_not_overwrite_existing_server() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, CONFIG).expect("写配置失败");
        seed_file(&state, &path, false);

        let mut remote = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, "cfg:remote"))
            .expect("查询失败")
            .expect("应存在");
        remote.name = "用户改名".to_string();
        state
            .db
            .with_write_blocking(|conn| server_repo::upsert(conn, &remote))
            .expect("更新失败");

        let report = seed_file(&state, &path, true);
        assert_eq!(report.added, 0);
        let loaded = state
            .db
            .with_read_blocking(|conn| server_repo::get(conn, "cfg:remote"))
            .expect("查询失败")
            .expect("应存在");
        assert_eq!(loaded.name, "用户改名");
    }

    #[test]
    fn invalid_entry_is_partial_and_error_is_reported() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(
            &path,
            r#"{"servers":{"bad":{"type":"http"},"ok":{"type":"http","url":"http://127.0.0.1:1/mcp"}}}"#,
        )
        .expect("写配置失败");
        let report = seed_file(&state, &path, false);
        assert_eq!(report.status, "partial");
        assert_eq!(report.added, 1);
        assert_eq!(report.failed.len(), 1);
        let bootstrap = state
            .db
            .with_read_blocking(bootstrap_state::load)
            .expect("读取状态失败");
        assert_eq!(bootstrap.last_status, "partial");
        assert!(bootstrap.last_error.unwrap().contains("bad"));
    }

    #[test]
    fn malformed_json_sets_error_status() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "{ not json").expect("写配置失败");
        let report = seed_file(&state, &path, false);
        assert_eq!(report.status, "error");
        let bootstrap = state
            .db
            .with_read_blocking(bootstrap_state::load)
            .expect("读取状态失败");
        assert_eq!(bootstrap.last_status, "error");
        assert!(bootstrap.last_error.unwrap().contains("JSON 解析失败"));
    }

    #[test]
    fn status_reports_changed_and_default() {
        let (dir, state) = test_state();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, CONFIG).expect("写配置失败");
        seed_file(&state, &path, false);

        let current = status(&state, None);
        assert_eq!(current.status, "applied");
        assert!(!current.changed);
        assert_eq!(current.default_server_id.as_deref(), Some("cfg:remote"));

        std::fs::write(&path, CONFIG.replace("8080", "8081")).expect("写配置失败");
        // 真实启动流程会先跑一次自动 seed；它不应把新 hash 写回去
        seed_file(&state, &path, false);
        let changed = status(&state, None);
        assert!(changed.changed);
    }

    #[test]
    fn missing_file_sets_error_status() {
        let (dir, state) = test_state();
        let path = dir.path().join("missing.json");
        let report = seed_file(&state, &path, false);
        assert_eq!(report.status, "error");
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].key, "__config__");
    }
}
