//! 调用命令（工具 / 资源 / 提示词 / 健康探测 / 取消）
//!
//! 每次调用完成后落库（call_repo）并通过 `history://recorded` 事件通知前端。

use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::commands::AppState;
use crate::mcp::models::{CallRecord, CallRecordSummary, ProbeResult};
use crate::storage::call_repo;

/// 调用工具（可选预置 call_id：UI 先拿到 id 再发起，便于取消）
#[tauri::command]
pub async fn call_tool(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    call_id: Option<String>,
    name: String,
    arguments: Value,
) -> Result<CallRecord, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let record = handle.call_tool_with_id(call_id, &name, arguments).await;
    Ok(persist_call(&state, &app, &record))
}

/// 读取资源
#[tauri::command]
pub async fn read_resource(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    uri: String,
) -> Result<CallRecord, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let record = handle.read_resource(&uri).await;
    Ok(persist_call(&state, &app, &record))
}

/// 获取提示词
#[tauri::command]
pub async fn get_prompt(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    name: String,
    arguments: Value,
) -> Result<CallRecord, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let record = handle.get_prompt(&name, arguments).await;
    Ok(persist_call(&state, &app, &record))
}

/// 取消在途调用（按传输语义：中止等待 + 发送取消通知）
#[tauri::command]
pub async fn cancel_call(
    state: State<'_, AppState>,
    id: String,
    call_id: String,
) -> Result<bool, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    Ok(handle.cancel_call(&call_id).await)
}

/// 健康探测（现代世代幂等枚举；旧世代 ping；同样落库）
#[tauri::command]
pub async fn probe_server(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<ProbeResult, String> {
    let handle = state
        .manager
        .handle(&id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let result = handle.probe().await.map_err(|e| e.message())?;
    let record = handle.record_probe(&result).await;
    persist_call(&state, &app, &record);
    Ok(result)
}

/// 落库 + 事件推送（失败仅告警，不影响调用返回）。
///
/// 返回回写了 `truncated/size_bytes` 的记录，供前端/报告使用。
pub(crate) fn persist_call(state: &AppState, app: &AppHandle, record: &CallRecord) -> CallRecord {
    let results_dir = state.data_dir.join("results");
    let mut updated = record.clone();
    match state
        .db
        .with_write_blocking(|conn| call_repo::insert(conn, &results_dir, record))
    {
        Ok(outcome) => {
            updated.truncated = outcome.truncated;
            updated.size_bytes = outcome.size_bytes;
        }
        Err(e) => {
            tracing::warn!(record_id = %record.id, error = %e, "调用记录落库失败");
            return updated;
        }
    }
    let summary = CallRecordSummary {
        id: updated.id.clone(),
        server_id: updated.server_id.clone(),
        server_name: updated.server_name.clone(),
        method: updated.method.clone(),
        target: updated.target.clone(),
        status: updated.status.clone(),
        duration_ms: updated.duration_ms,
        size_bytes: updated.size_bytes,
        truncated: updated.truncated,
        origin: updated.origin.clone(),
        created_at: updated.created_at,
    };
    let _ = app.emit("history://recorded", summary);
    updated
}
