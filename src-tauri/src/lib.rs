//! MCP Tester - MCP 服务接口测试桌面工具
//!
//! 模块划分见各模块注释。

mod bootstrap;
mod commands;
pub mod core;
mod credentials;
mod logging;
pub mod mcp;
mod storage;

use std::sync::Arc;

use tauri::Manager;

use commands::AppState;
use mcp::models::RuntimeSettings;

/// setup 闭包错误转换辅助
fn setup_error(e: impl std::fmt::Display) -> Box<dyn std::error::Error> {
    e.to_string().into()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 运行目录：便携模式（exe 旁 mcptester.portable）/ MCPTESTER_DATA_DIR / AppData
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
                .ok_or_else(|| "无法定位程序目录".to_string())
                .map_err(setup_error)?;
            let app_data_dir = app.path().app_data_dir().map_err(setup_error)?;
            let app_local_data_dir = app.path().app_local_data_dir().map_err(setup_error)?;
            let env_data_dir = std::env::var_os("MCPTESTER_DATA_DIR").map(std::path::PathBuf::from);
            let paths = bootstrap::paths::resolve(
                &exe_dir,
                &app_data_dir,
                &app_local_data_dir,
                env_data_dir.as_deref(),
            )
            .map_err(setup_error)?;
            if let Some(warning) = &paths.warning {
                tracing::warn!(warning = %warning, "启动目录回退");
            }
            std::fs::create_dir_all(&paths.data_dir).map_err(setup_error)?;
            std::fs::create_dir_all(&paths.log_dir).map_err(setup_error)?;

            // 日志目录（便携模式下位于 data/logs）
            let log_guard = logging::init(&paths.log_dir).map_err(setup_error)?;
            app.manage(log_guard);

            // 凭据存储：初始化平台凭据库并自检持久化能力
            if let Err(e) = credentials::init_store() {
                tracing::warn!("凭据存储初始化失败：{e}");
            }
            tracing::info!(status = ?credentials::persistence_status(), "凭据存储状态自检");

            // 数据库：打开 + 版本化迁移
            let db = storage::Database::open(&paths.data_dir.join("mcptester.db"))
                .map_err(setup_error)?;
            db.migrate().map_err(setup_error)?;

            let state = AppState {
                db,
                manager: Arc::new(mcp::ConnectionManager::new(RuntimeSettings::default())),
                data_dir: paths.data_dir.clone(),
                log_dir: paths.log_dir.clone(),
                portable: paths.portable,
                data_dir_source: paths.source.as_str().to_string(),
            };

            // 启动配置 seed：只补充缺失项，不覆盖用户修改
            let resource_dir = app.path().resource_dir().ok();
            let report = bootstrap::config_file::seed(&state, resource_dir.as_deref(), false);
            match report.status.as_str() {
                "applied" => tracing::info!(
                    path = ?report.path,
                    added = report.added,
                    skipped = report.skipped,
                    "启动配置已应用"
                ),
                "partial" | "error" => tracing::warn!(
                    path = ?report.path,
                    added = report.added,
                    failed = report.failed.len(),
                    "启动配置应用失败或不完整"
                ),
                _ => tracing::info!(path = ?report.path, "未发现启动配置文件"),
            }

            // 启动自检：重试待清理的 keyring 条目 + 清理孤儿索引
            commands::server::startup_secret_check(&state);

            // 启动保留清理（历史天数 / 条数 / 落盘目录上限）
            run_retention_cleanup(&state);

            app.manage(state);

            // 前端事件转发（连接状态 / 通知 / stderr）
            commands::events::spawn_event_forwarder(app.handle().clone());

            // 启动配置要求时，后台连接默认服务器
            bootstrap::config_file::spawn_auto_connect(app.handle().clone());

            tracing::info!(version = env!("CARGO_PKG_VERSION"), "MCP Tester 启动完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::get_app_info,
            commands::system::open_data_dir,
            commands::system::open_config_dir,
            commands::bootstrap::reload_bootstrap_config,
            commands::server::list_servers,
            commands::server::get_server,
            commands::server::add_server,
            commands::server::update_server,
            commands::server::update_server_meta,
            commands::server::delete_server,
            commands::server::connect_server,
            commands::server::disconnect_server,
            commands::server::reconnect_server,
            commands::server::get_connection_states,
            commands::capability::get_capabilities,
            commands::capability::refresh_capabilities,
            commands::events::get_wire_log,
            commands::events::clear_wire_log,
            commands::events::export_wire_log,
            commands::call::call_tool,
            commands::call::read_resource,
            commands::call::get_prompt,
            commands::call::cancel_call,
            commands::call::probe_server,
            commands::history::list_call_records,
            commands::history::get_call_record,
            commands::history::get_full_result_path,
            commands::history::read_full_result,
            commands::history::delete_call_records,
            commands::history::clear_call_records,
            commands::history::export_call_records,
            commands::history::preview_export_call_records,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::import_export::import_servers,
            commands::import_export::import_servers_from_file,
            commands::import_export::export_servers,
            commands::import_export::preview_export_servers,
            commands::import_export::get_server_json,
            commands::server::test_server_connection,
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 启动保留清理（失败仅告警）
fn run_retention_cleanup(state: &AppState) {
    let settings = state
        .db
        .with_read_blocking(storage::settings_repo::get)
        .unwrap_or_default();
    let results_dir = state.data_dir.join("results");
    match state.db.with_write_blocking(|conn| {
        storage::call_repo::cleanup(
            conn,
            &results_dir,
            settings.history_retention_days,
            settings.history_max_rows,
        )
    }) {
        Ok((records, files)) if records > 0 || files > 0 => {
            tracing::info!(records, files, "历史保留清理完成");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "历史保留清理失败"),
    }
}
