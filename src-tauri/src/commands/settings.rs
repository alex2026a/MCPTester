//! 设置命令

use tauri::State;

use crate::commands::AppState;
use crate::storage::settings_repo::{self, Settings};

/// 读取设置
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state
        .db
        .with_read_blocking(settings_repo::get)
        .map_err(|e| e.to_string())
}

/// 更新设置（超时/保留等将在下次连接或下次清理时生效）
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let settings_clone = settings.clone();
    state
        .db
        .with_write_blocking(move |conn| settings_repo::update(conn, &settings_clone))
        .map_err(|e| e.to_string())?;
    Ok(settings)
}
