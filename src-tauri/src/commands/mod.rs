//! Tauri command 层（按 server / capability / call / events / system 拆分）

pub mod bootstrap;
pub mod call;
pub mod capability;
pub mod events;
pub mod history;
pub mod import_export;
pub mod server;
pub mod settings;
pub mod system;

use std::path::PathBuf;
use std::sync::Arc;

use crate::mcp::ConnectionManager;
use crate::storage::Database;

/// 应用级共享状态（由 lib.rs setup 注册）
pub struct AppState {
    pub db: Database,
    pub manager: Arc<ConnectionManager>,
    /// 数据目录（results/ 落盘、导出默认路径）
    pub data_dir: PathBuf,
    /// 日志目录（便携模式下位于数据目录下）
    pub log_dir: PathBuf,
    /// 是否处于便携模式（exe 旁存在 mcptester.portable 且数据目录可写）
    pub portable: bool,
    /// 数据目录来源：env / portable / appdata / fallback
    pub data_dir_source: String,
}
