//! 启动配置相关命令。

use tauri::{AppHandle, Manager, State};

use crate::bootstrap::config_file::{self, SeedReport};
use crate::commands::AppState;

/// 手动重新应用启动配置：只补回缺失的预置服务器，不覆盖已有服务器。
#[tauri::command]
pub async fn reload_bootstrap_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SeedReport, String> {
    let resource_dir = app.path().resource_dir().ok();
    let report = config_file::seed(&state, resource_dir.as_deref(), true);
    if report.auto_connect {
        config_file::spawn_auto_connect(app.clone());
    }
    Ok(report)
}
