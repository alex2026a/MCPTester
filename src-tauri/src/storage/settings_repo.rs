//! 设置仓储（settings 表单行 JSON；见 04 文档第 8 节）

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 应用设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    pub history_retention_days: u32,
    pub history_max_rows: u32,
    pub stderr_buffer_lines: usize,
    pub notification_buffer_lines: usize,
    pub auto_reconnect: bool,
    pub auto_refresh_on_list_changed: bool,
    #[serde(default = "default_test_call_retention_days")]
    pub test_call_retention_days: u32,
    #[serde(default = "default_test_call_max_rows")]
    pub test_call_max_rows: u32,
    #[serde(default = "default_test_run_retention_days")]
    pub test_run_retention_days: u32,
    #[serde(default = "default_test_run_max_rows")]
    pub test_run_max_rows: u32,
}

fn default_test_call_retention_days() -> u32 {
    30
}

fn default_test_call_max_rows() -> u32 {
    5000
}

fn default_test_run_retention_days() -> u32 {
    90
}

fn default_test_run_max_rows() -> u32 {
    100
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            request_timeout_secs: 30,
            connect_timeout_secs: 15,
            history_retention_days: 30,
            history_max_rows: 10000,
            stderr_buffer_lines: 2000,
            notification_buffer_lines: 1000,
            auto_reconnect: true,
            auto_refresh_on_list_changed: true,
            test_call_retention_days: default_test_call_retention_days(),
            test_call_max_rows: default_test_call_max_rows(),
            test_run_retention_days: default_test_run_retention_days(),
            test_run_max_rows: default_test_run_max_rows(),
        }
    }
}

/// 读取设置（缺省返回默认值）
pub fn get(conn: &Connection) -> anyhow::Result<Settings> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |row| row.get(0))
        .optional()?;
    match json {
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        None => Ok(Settings::default()),
    }
}

/// 写入设置
pub fn update(conn: &Connection, settings: &Settings) -> anyhow::Result<()> {
    let json = serde_json::to_string(settings)?;
    conn.execute(
        "INSERT INTO settings (id, json) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        [json],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_defaults() {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let mut conn = rusqlite::Connection::open(dir.path().join("test.db")).expect("打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");

        let defaults = get(&conn).expect("读取失败");
        assert_eq!(defaults.request_timeout_secs, 30);

        let custom = Settings {
            request_timeout_secs: 5,
            history_retention_days: 7,
            ..Default::default()
        };
        update(&conn, &custom).expect("写入失败");
        let loaded = get(&conn).expect("读取失败");
        assert_eq!(loaded.request_timeout_secs, 5);
        assert_eq!(loaded.history_retention_days, 7);
    }

    #[test]
    fn old_settings_json_keeps_values_and_defaults_new_fields() {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let mut conn = rusqlite::Connection::open(dir.path().join("test.db")).expect("打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");
        // v0.2.0 及更早的 settings JSON（不含测试保留字段）
        conn.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)",
            [r#"{"request_timeout_secs":9,"connect_timeout_secs":3,"history_retention_days":5,"history_max_rows":77,"stderr_buffer_lines":100,"notification_buffer_lines":50,"auto_reconnect":false,"auto_refresh_on_list_changed":false}"#],
        )
        .expect("写入失败");

        let loaded = get(&conn).expect("读取失败");
        assert_eq!(loaded.request_timeout_secs, 9);
        assert_eq!(loaded.history_retention_days, 5);
        assert!(!loaded.auto_reconnect);
        assert_eq!(loaded.test_call_retention_days, 30);
        assert_eq!(loaded.test_run_max_rows, 100);
    }
}
