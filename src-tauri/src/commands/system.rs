//! 系统类命令：应用信息与目录操作（M1 打通 IPC 链路）

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::bootstrap::config_file;
use crate::commands::AppState;

/// 客户端支持的协议版本清单（与 02 文档 1.1 对齐，新版本在前）
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// 应用信息（前端 AppInfo 契约，见 04 文档第 7 节）
#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub log_dir: String,
    pub supported_protocol_versions: Vec<String>,
    /// 是否便携模式（数据在 exe 同目录 data/）
    pub portable: bool,
    /// 数据目录来源：env / portable / appdata / fallback
    pub data_dir_source: String,
    /// 启动配置文件路径（未找到时为 None）
    pub config_path: Option<String>,
    /// not_found / pending / applied / partial / error
    pub config_status: String,
    pub config_error: Option<String>,
    /// 配置文件在 seed 后是否发生变化（seed 模式不会自动覆盖）
    pub config_changed: bool,
    pub default_server_id: Option<String>,
    pub auto_connect: bool,
}

/// 获取应用信息（版本、数据目录、日志目录、启动配置、协议支持清单）
#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> Result<AppInfo, String> {
    let resource_dir = app.path().resource_dir().ok();
    let config = config_file::status(&state, resource_dir.as_deref());

    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.data_dir.display().to_string(),
        log_dir: state.log_dir.display().to_string(),
        supported_protocol_versions: SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        portable: state.portable,
        data_dir_source: state.data_dir_source.clone(),
        config_path: config.path,
        config_status: config.status,
        config_error: config.error,
        config_changed: config.changed,
        default_server_id: config.default_server_id,
        auto_connect: config.auto_connect,
    })
}

/// 调起系统文件管理器打开数据目录
#[tauri::command]
pub fn open_data_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    app.opener()
        .open_path(state.data_dir.display().to_string(), None::<&str>)
        .map_err(|e| format!("打开数据目录失败：{e}"))
}

/// 调起系统文件管理器打开启动配置文件所在目录；未找到配置时打开数据目录。
#[tauri::command]
pub fn open_config_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let resource_dir = app.path().resource_dir().ok();
    let config = config_file::status(&state, resource_dir.as_deref());
    let target = config
        .path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).parent())
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| state.data_dir.clone());

    app.opener()
        .open_path(target.display().to_string(), None::<&str>)
        .map_err(|e| format!("打开配置目录失败：{e}"))
}
