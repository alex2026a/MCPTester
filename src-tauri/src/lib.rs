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
pub mod testing;

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
                test_run: Arc::new(tokio::sync::Mutex::new(None)),
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

            // 回收上次异常退出留下的测试运行
            if let Err(e) = state
                .db
                .with_write_blocking(|conn| storage::test_repo::recover_interrupted_runs(conn))
            {
                tracing::warn!(error = %e, "测试运行回收失败");
            }

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
            commands::testing::list_test_suites,
            commands::testing::get_test_suite,
            commands::testing::create_test_suite,
            commands::testing::update_test_suite,
            commands::testing::rebind_test_suite,
            commands::testing::delete_test_suite,
            commands::testing::list_test_cases,
            commands::testing::create_test_cases,
            commands::testing::update_test_case,
            commands::testing::delete_test_cases,
            commands::testing::duplicate_test_case,
            commands::testing::reorder_test_cases,
            commands::testing::generate_test_cases,
            commands::testing::save_generated_cases,
            commands::testing::run_test_suite,
            commands::testing::run_test_cases,
            commands::testing::cancel_test_run,
            commands::testing::get_active_test_run,
            commands::testing::list_test_runs,
            commands::testing::get_test_run,
            commands::testing::list_test_run_cases,
            commands::testing::get_test_run_case,
            commands::testing::set_test_baseline,
            commands::testing::compare_test_runs,
            commands::testing::rerun_failed_test_cases,
            commands::testing::export_test_report,
            commands::testing::get_contract_snapshot,
            commands::testing::diff_contract,
            commands::testing::save_call_as_case,
            commands::testing::export_test_suite,
            commands::testing::import_test_cases,
            commands::testing::import_llm_test_cases,
            commands::testing::export_llm_prompt,
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
    let reports_dir = state.data_dir.join("reports");
    match state.db.with_write_blocking(|conn| {
        let (manual, files_a) = storage::call_repo::cleanup_origin(
            conn,
            &results_dir,
            Some("manual"),
            settings.history_retention_days,
            settings.history_max_rows,
        )?;
        let (test, files_b) = storage::call_repo::cleanup_origin(
            conn,
            &results_dir,
            Some("test"),
            settings.test_call_retention_days,
            settings.test_call_max_rows,
        )?;
        let runs = storage::test_repo::cleanup_runs(
            conn,
            &reports_dir,
            settings.test_run_retention_days,
            settings.test_run_max_rows,
        )?;
        Ok((manual + test, files_a + files_b, runs))
    }) {
        Ok((records, files, runs)) if records > 0 || files > 0 || runs > 0 => {
            tracing::info!(records, files, runs, "历史/测试保留清理完成");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "历史/测试保留清理失败"),
    }
}
