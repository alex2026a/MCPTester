//! 前端事件转发：连接状态、服务端通知、stderr、进度（见 04 文档第 2 节）
//!
//! 实现方式：轮询管理器中的连接状态（变化即推送）+ 为每个连接订阅 EventHub 广播。
//! v0.1.3 起同时增量推送报文日志（`mcp://wire`），前端不再轮询全量拉取。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::AppState;
use crate::mcp::handler::HubEvent;
use crate::mcp::wire::WireEntry;

/// 读取指定服务器的线上报文日志（v0.1.1；未连接时返回空列表）
#[tauri::command]
pub async fn get_wire_log(
    state: tauri::State<'_, AppState>,
    server_id: String,
) -> Result<Vec<WireEntry>, String> {
    match state.manager.handle(&server_id).await {
        Some(handle) => Ok(handle.wire().buffer()),
        None => Ok(Vec::new()),
    }
}

/// 清空指定服务器的报文日志（v0.1.3）
#[tauri::command]
pub async fn clear_wire_log(
    state: tauri::State<'_, AppState>,
    server_id: String,
) -> Result<bool, String> {
    match state.manager.handle(&server_id).await {
        Some(handle) => {
            handle.wire().clear();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 导出指定服务器的报文日志（v0.1.3；返回写入路径）
#[tauri::command]
pub async fn export_wire_log(
    state: tauri::State<'_, AppState>,
    server_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let entries = match state.manager.handle(&server_id).await {
        Some(handle) => handle.wire().buffer(),
        None => Vec::new(),
    };
    let target: PathBuf = match path {
        Some(path) => PathBuf::from(path),
        None => state.data_dir.join("wire").join(format!(
            "wire-{}-{}.json",
            server_id,
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        )),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    std::fs::write(&target, json).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// 启动事件转发任务（应用启动时调用一次）
pub fn spawn_event_forwarder(app: AppHandle) {
    // OAuth 登录事件（v0.3.0）：broadcast -> mcp://oauth
    {
        let app = app.clone();
        let mut receiver = app.state::<AppState>().oauth.subscribe();
        tauri::async_runtime::spawn(async move {
            while let Ok(event) = receiver.recv().await {
                let _ = app.emit("mcp://oauth", event);
            }
        });
    }

    tauri::async_runtime::spawn(async move {
        let mut subscribed: HashSet<String> = HashSet::new();
        let mut last_state: HashMap<String, String> = HashMap::new();
        let mut wire_cursor: HashMap<String, u64> = HashMap::new();

        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;

            let (db_handles, manager) = {
                let state = app.state::<AppState>();
                (
                    state.manager.all_handles().await,
                    Arc::clone(&state.manager),
                )
            };
            let _ = db_handles;

            for (id, handle) in manager.all_handles().await {
                // 连接状态变化推送
                let current_state = handle.state().await;
                let serialized = serde_json::to_string(&current_state).unwrap_or_default();
                if last_state.get(&id) != Some(&serialized) {
                    last_state.insert(id.clone(), serialized);
                    let _ = app.emit(
                        "mcp://connection-state",
                        json!({ "server_id": id, "state": current_state }),
                    );
                }

                // 报文增量推送（v0.1.3：替代前端轮询）
                let latest = handle.wire().latest_seq();
                let cursor = wire_cursor.entry(id.clone()).or_insert(0);
                if latest < *cursor {
                    // 连接重建后 seq 从 1 重新计数
                    *cursor = 0;
                }
                if latest > *cursor {
                    let entries = handle.wire().since(*cursor);
                    let dropped = entries
                        .first()
                        .map(|entry| entry.seq > *cursor + 1)
                        .unwrap_or(false);
                    *cursor = latest;
                    let _ = app.emit(
                        "mcp://wire",
                        json!({ "server_id": id, "entries": entries, "dropped": dropped }),
                    );
                }

                // 每个连接订阅一次事件流
                if subscribed.insert(id.clone()) {
                    let mut rx = handle.hub().subscribe();
                    let app = app.clone();
                    let server_id = id.clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            match rx.recv().await {
                                Ok(HubEvent::Notification(entry)) => {
                                    let _ = app.emit(
                                        "mcp://notification",
                                        json!({
                                            "server_id": server_id,
                                            "seq": entry.seq,
                                            "method": entry.method,
                                            "params": entry.params,
                                            "at": entry.at,
                                        }),
                                    );
                                }
                                Ok(HubEvent::Stderr(line)) => {
                                    let _ = app.emit(
                                        "mcp://stderr",
                                        json!({
                                            "server_id": server_id,
                                            "seq": line.seq,
                                            "line": line.line,
                                            "at": line.at,
                                        }),
                                    );
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue;
                                }
                                Err(_) => break,
                            }
                        }
                    });
                }
            }
        }
    });
}
