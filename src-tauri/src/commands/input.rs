//! 交互输入命令（v0.6.0）

use tauri::State;

use crate::commands::AppState;
use crate::mcp::input::{InputAnswer, InputEvent};

/// 提交输入应答；校验失败时保留 pending，前端可修正后重试
#[tauri::command]
pub async fn respond_input(
    state: State<'_, AppState>,
    request_id: String,
    answers: Vec<InputAnswer>,
) -> Result<(), String> {
    state
        .input
        .respond(&request_id, answers)
        .map_err(|error| error.to_string())
}

/// 取消一次输入所属的调用；若 pending 已消失则视为已完成
#[tauri::command]
pub async fn cancel_input(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    let Some(context) = state.input.cancel(&request_id) else {
        return Ok(());
    };
    if let Some(call_id) = context.call_id
        && let Some(handle) = state.manager.handle(&context.server_id).await
    {
        let _ = handle.cancel_call(&call_id).await;
    }
    Ok(())
}

/// 当前挂起的输入请求（HMR/刷新后恢复）
#[tauri::command]
pub async fn list_pending_inputs(state: State<'_, AppState>) -> Result<Vec<InputEvent>, String> {
    Ok(state.input.list_pending())
}
