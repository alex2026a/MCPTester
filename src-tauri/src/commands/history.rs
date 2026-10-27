//! 历史记录命令：列表 / 详情 / 删除 / 清空 / 导出

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::commands::AppState;
use crate::storage::call_repo::{self, CallFilter};
use crate::mcp::models::{
    CallMethod, CallOrigin, CallRecord, CallRecordSummary, CallStatus,
};

/// 前端筛选参数
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallFilterInput {
    pub server_id: Option<String>,
    pub method: Option<String>,
    pub target: Option<String>,
    pub status: Option<String>,
    /// manual / test；None = 全部
    pub origin: Option<String>,
    pub keyword: Option<String>,
    #[serde(default)]
    pub keyword_full: bool,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl From<CallFilterInput> for CallFilter {
    fn from(value: CallFilterInput) -> Self {
        CallFilter {
            server_id: value.server_id,
            method: value.method,
            target: value.target,
            status: value.status,
            origin: value.origin,
            keyword: value.keyword,
            keyword_full: value.keyword_full,
            from: value.from,
            to: value.to,
        }
    }
}

/// 分页结果
#[derive(Debug, Clone, Serialize)]
pub struct PagedCallRecords {
    pub items: Vec<CallRecordSummary>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// 列表（筛选 + 分页）
#[tauri::command]
pub async fn list_call_records(
    state: State<'_, AppState>,
    filter: CallFilterInput,
    offset: u64,
    limit: u64,
) -> Result<PagedCallRecords, String> {
    let filter: CallFilter = filter.into();
    let limit = limit.clamp(1, 500);
    state
        .db
        .with_read_blocking(|conn| call_repo::list(conn, &filter, offset, limit))
        .map(|(items, total)| PagedCallRecords {
            items,
            total,
            offset,
            limit,
        })
        .map_err(|e| e.to_string())
}

/// 接口页“最近参数”列表项。
#[derive(Debug, Clone, Serialize)]
pub struct RecentCallParam {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub status: CallStatus,
    pub duration_ms: u64,
    pub origin: CallOrigin,
    pub preview: String,
    pub param_count: u32,
    pub repeat_count: u32,
    pub params_bytes: u64,
}

/// 某接口最近发送过的参数（默认只看手工调用）。
#[tauri::command]
pub async fn list_recent_call_params(
    state: State<'_, AppState>,
    server_id: String,
    method: CallMethod,
    target: String,
    limit: Option<u64>,
    include_test: Option<bool>,
) -> Result<Vec<RecentCallParam>, String> {
    state
        .db
        .with_read_blocking(move |conn| {
            call_repo::recent_params(
                conn,
                &server_id,
                method.as_str(),
                &target,
                include_test.unwrap_or(false),
                limit.unwrap_or(20),
            )
        })
        .map(|items| {
            items
                .into_iter()
                .map(|item| RecentCallParam {
                    id: item.id,
                    created_at: item.created_at,
                    status: item.status,
                    duration_ms: item.duration_ms,
                    origin: item.origin,
                    preview: item.preview,
                    param_count: item.param_count,
                    repeat_count: item.repeat_count,
                    params_bytes: item.params_bytes,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// 只读取一次调用的参数，不读取结果。
#[tauri::command]
pub async fn get_call_params(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<Value, String> {
    state
        .db
        .with_read_blocking(move |conn| call_repo::get_params(conn, &record_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "记录不存在".to_string())
}

/// 详情（含完整参数与结果）
#[tauri::command]
pub async fn get_call_record(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<CallRecord, String> {
    state
        .db
        .with_read_blocking(|conn| call_repo::get(conn, &record_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "记录不存在".to_string())
}

/// 删除指定记录（同步清理落盘文件）
#[tauri::command]
pub async fn delete_call_records(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<u64, String> {
    let results_dir = results_dir(&state);
    state
        .db
        .with_write_blocking(move |conn| call_repo::delete(conn, &results_dir, &ids))
        .map_err(|e| e.to_string())
}

/// 清空（按筛选条件）
#[tauri::command]
pub async fn clear_call_records(
    state: State<'_, AppState>,
    filter: CallFilterInput,
) -> Result<u64, String> {
    let results_dir = results_dir(&state);
    let filter: CallFilter = filter.into();
    state
        .db
        .with_write_blocking(move |conn| call_repo::clear(conn, &results_dir, &filter))
        .map_err(|e| e.to_string())
}

/// 导出预览（v0.1.1：返回 JSON 文本与条数，不落盘）
#[tauri::command]
pub async fn preview_export_call_records(
    state: State<'_, AppState>,
    filter: CallFilterInput,
    include_frames: Option<bool>,
) -> Result<ExportPreview, String> {
    let filter: CallFilter = filter.into();
    let include_frames = include_frames.unwrap_or(false);
    state
        .db
        .with_read_blocking(move |conn| {
            call_repo::export_text(conn, &filter, include_frames)
        })
        .map(|(json, count)| ExportPreview { json, count })
        .map_err(|e| e.to_string())
}

/// 导出预览载荷
#[derive(Debug, Clone, Serialize)]
pub struct ExportPreview {
    pub json: String,
    pub count: u64,
}

/// 导出（写入用户选择/默认路径）
#[tauri::command]
pub async fn export_call_records(
    state: State<'_, AppState>,
    filter: CallFilterInput,
    path: Option<String>,
    include_frames: Option<bool>,
) -> Result<String, String> {
    let filter: CallFilter = filter.into();
    let include_frames = include_frames.unwrap_or(false);
    let target: PathBuf = match path {
        Some(path) => PathBuf::from(path),
        None => results_dir(&state).join(format!(
            "history-export-{}.json",
            Utc::now().format("%Y%m%d-%H%M%S")
        )),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let path_clone = target.clone();
    state
        .db
        .with_read_blocking(move |conn| {
            call_repo::export(conn, &filter, &path_clone, include_frames)
        })
        .map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// 大结果全文路径（v0.1.3：>1MB 结果落盘于 `results/<id>.json`；未落盘时为 null）
#[tauri::command]
pub async fn get_full_result_path(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<Option<String>, String> {
    let path = result_file_path(&results_dir(&state), &record_id)?;
    Ok(path.exists().then(|| path.display().to_string()))
}

/// 读取大结果全文（v0.1.3：截断入库的完整内容）
#[tauri::command]
pub async fn read_full_result(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<String, String> {
    let path = result_file_path(&results_dir(&state), &record_id)?;
    std::fs::read_to_string(&path).map_err(|e| format!("读取完整结果失败：{e}"))
}

/// 结果文件路径（校验记录 ID 形态，防目录穿越）
fn result_file_path(dir: &Path, record_id: &str) -> Result<PathBuf, String> {
    let valid = !record_id.is_empty()
        && record_id.len() <= 64
        && record_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err("非法的记录 ID".to_string());
    }
    Ok(dir.join(format!("{record_id}.json")))
}

fn results_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("results")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_file_path_rejects_traversal() {
        let dir = Path::new("C:/data/results");
        assert!(result_file_path(dir, "../secret").is_err());
        assert!(result_file_path(dir, "a/b").is_err());
        assert!(result_file_path(dir, "").is_err());
        assert!(result_file_path(dir, "\\windows\\system32").is_err());
        assert!(result_file_path(dir, "..").is_err());
    }

    #[test]
    fn result_file_path_joins_results_dir() {
        let dir = Path::new("C:/data/results");
        let path = result_file_path(dir, "abc-123").expect("合法 ID");
        assert_eq!(path, dir.join("abc-123.json"));
    }
}
