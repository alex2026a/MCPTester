//! 启动配置 seed 状态（持久化在 `app_meta` 表）。

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::storage::app_meta;

/// `app_meta` 中的键
pub const META_KEY: &str = "bootstrap";

/// 已应用配置文件条目（key -> server id）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootstrapState {
    /// 已应用过的配置 key；一旦记录，自动 seed 不再重复创建
    #[serde(default)]
    pub applied: BTreeMap<String, String>,
    /// 默认服务器 ID（接口页默认选中 / 自动连接）
    #[serde(default)]
    pub default_server_id: Option<String>,
    /// 是否在启动时自动连接默认服务器
    #[serde(default)]
    pub auto_connect: bool,
    /// 最近一次读取的配置文件路径
    #[serde(default)]
    pub path: Option<String>,
    /// 最近一次成功解析的文件内容哈希
    #[serde(default)]
    pub hash: Option<String>,
    /// 最近一次应用时间
    #[serde(default)]
    pub applied_at: Option<DateTime<Utc>>,
    /// applied / partial / error
    #[serde(default)]
    pub last_status: String,
    /// 最近一次错误摘要（不含敏感值）
    #[serde(default)]
    pub last_error: Option<String>,
}

/// 读取状态（损坏时回退默认值并告警，不阻塞启动）
pub fn load(conn: &Connection) -> anyhow::Result<BootstrapState> {
    let Some(json) = app_meta::get(conn, META_KEY)? else {
        return Ok(BootstrapState::default());
    };
    match serde_json::from_str(&json) {
        Ok(state) => Ok(state),
        Err(e) => {
            tracing::warn!(error = %e, "启动配置状态损坏，按空状态处理");
            Ok(BootstrapState::default())
        }
    }
}

/// 写入状态
pub fn save(conn: &Connection, state: &BootstrapState) -> anyhow::Result<()> {
    let json = serde_json::to_string(state)?;
    app_meta::set(conn, META_KEY, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("内存库打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");

        let mut state = BootstrapState::default();
        state
            .applied
            .insert("demo".to_string(), "cfg:demo".to_string());
        state.default_server_id = Some("cfg:demo".to_string());
        state.auto_connect = true;
        state.last_status = "applied".to_string();
        save(&conn, &state).expect("写入失败");

        let loaded = load(&conn).expect("读取失败");
        assert_eq!(
            loaded.applied.get("demo").map(String::as_str),
            Some("cfg:demo")
        );
        assert_eq!(loaded.default_server_id.as_deref(), Some("cfg:demo"));
        assert!(loaded.auto_connect);
        assert_eq!(loaded.last_status, "applied");
    }
}
