//! 能力枚举命令（Tools / Resources / Templates / Prompts）

use tauri::State;

use crate::commands::AppState;
use crate::mcp::models::CapabilitySnapshot;

/// 获取能力快照（缓存优先；无缓存时拉取）
#[tauri::command]
pub async fn get_capabilities(
    state: State<'_, AppState>,
    id: String,
) -> Result<CapabilitySnapshot, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    handle.capabilities(false).await.map_err(|e| e.message())
}

/// 强制重拉能力快照（分页全量；失败回退上次快照并标注过期）
#[tauri::command]
pub async fn refresh_capabilities(
    state: State<'_, AppState>,
    id: String,
) -> Result<CapabilitySnapshot, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    handle.capabilities(true).await.map_err(|e| e.message())
}
