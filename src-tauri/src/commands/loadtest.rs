//! v0.7.0 压测命令：预览、启动、取消、历史、导出。

use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::{AppState, RunKind, RunSlotGuard, clear_run_slot};
use crate::loadtest::engine::{LoadContext, LoadEventSink, run as run_engine};
use crate::loadtest::models::{
    ActiveLoadRun, LoadCaseSnapshot, LoadManifest, LoadPreview, LoadPreviewOperation, LoadProfile,
    LoadRiskSummary, LoadRun, LoadRunControl, LoadRunStatus, LoadRunSummary, LoadSummary,
};
use crate::loadtest::validate::{
    preview_operation, snapshot_operation, summarize_risks, validate_profile,
};
use crate::mcp::models::{ConnectionState, ServerConfig};
use crate::storage::{load_repo, test_repo};
use crate::testing::models::Paged;
use crate::testing::runner::{effective_risk, target_exists};

struct PreparedLoadData {
    config: ServerConfig,
    server_name: String,
    suite_id: String,
    suite_name: String,
    operations: Vec<LoadCaseSnapshot>,
    preview: Vec<LoadPreviewOperation>,
    risk_summary: LoadRiskSummary,
    production: bool,
    protocol_version: String,
    lifecycle: String,
    transport: String,
    capability_hash: String,
    capability_stale: bool,
    tool_count: u64,
    resource_count: u64,
    template_count: u64,
    prompt_count: u64,
}

/// 启动前预览（不建 run、不占活动锁；用缓存能力快照）。
#[tauri::command]
pub async fn preview_load_test(
    state: State<'_, AppState>,
    server_id: String,
    suite_id: String,
    profile: LoadProfile,
) -> Result<LoadPreview, String> {
    let prepared = prepare_load(&state, &server_id, &suite_id, &profile, false).await?;
    let warnings = if prepared.production {
        vec!["服务器带生产标签；write/unknown 操作需要启动确认".to_string()]
    } else {
        Vec::new()
    };
    let can_start = prepared.preview.iter().all(|item| item.allowed);
    Ok(LoadPreview {
        server_id,
        suite_id,
        production: prepared.production,
        risk_summary: prepared.risk_summary,
        operations: prepared.preview,
        warnings,
        can_start,
    })
}

/// 启动压测（后台执行，返回 run_id）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_load_test(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: String,
    suite_id: String,
    profile: LoadProfile,
    confirm_production: bool,
) -> Result<String, String> {
    let mut slot = RunSlotGuard::reserve(state.run_slot.clone(), RunKind::Load)?;
    let prepared = prepare_load(&state, &server_id, &suite_id, &profile, true).await?;
    if !prepared.preview.iter().all(|item| item.allowed) {
        let first = prepared
            .preview
            .iter()
            .find(|item| !item.allowed)
            .map(|item| format!("{}：{}", item.name, reason_text(item.reason.as_deref())))
            .unwrap_or_else(|| "压测操作未通过风险校验".to_string());
        return Err(first);
    }
    if prepared.production
        && !confirm_production
        && prepared
            .preview
            .iter()
            .any(|item| matches!(item.effective_risk, crate::testing::models::TestRisk::Write | crate::testing::models::TestRisk::Unknown))
    {
        return Err("生产标签服务器执行 write/unknown 压测需要二次确认".to_string());
    }
    if state.test_run.lock().await.is_some() {
        return Err("已有测试正在运行，请先取消".to_string());
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let manifest = LoadManifest {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        server_id: server_id.clone(),
        server_name: prepared.server_name.clone(),
        transport: prepared.transport.clone(),
        protocol_version: prepared.protocol_version.clone(),
        lifecycle: prepared.lifecycle.clone(),
        capability_hash: prepared.capability_hash.clone(),
        capability_stale: prepared.capability_stale,
        tool_count: prepared.tool_count,
        resource_count: prepared.resource_count,
        template_count: prepared.template_count,
        prompt_count: prepared.prompt_count,
        production: prepared.production,
        risk_summary: prepared.risk_summary.clone(),
        started_at: now,
    };
    let run = LoadRun {
        id: run_id.clone(),
        server_id: server_id.clone(),
        server_name: prepared.server_name.clone(),
        suite_id: prepared.suite_id.clone(),
        suite_name: prepared.suite_name.clone(),
        profile_name: profile.name.clone(),
        status: LoadRunStatus::Running,
        stop_reason: None,
        profile: profile.clone(),
        operations: prepared.operations.clone(),
        manifest,
        summary: LoadSummary::default(),
        series: Vec::new(),
        errors: Vec::new(),
        started_at: now,
        finished_at: None,
    };
    let run_for_db = run.clone();
    state
        .db
        .with_write_blocking(move |conn| load_repo::insert_run(conn, &run_for_db))
        .map_err(|e| format!("创建压测运行失败：{e}"))?;

    {
        let mut guard = state.load_run.lock().await;
        if guard.is_some() {
            let _ = state
                .db
                .with_write_blocking(|conn| {
                    load_repo::finish_run(
                        conn,
                        &run_id,
                        LoadRunStatus::Failed,
                        Some("already_running"),
                        &LoadSummary::default(),
                        &[],
                        &[],
                        Utc::now(),
                    )
                });
            slot.release();
            return Err("已有压测正在运行，请先停止".to_string());
        }
        *guard = Some(LoadRunControl::new(run_id.clone(), server_id.clone()));
    }
    slot.commit(run_id.clone());

    let control = {
        let guard = state.load_run.lock().await;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "压测运行锁丢失".to_string())?
    };
    let db = state.db.clone();
    let app_for_task = app.clone();
    let app_for_events = app.clone();
    let event_sink: LoadEventSink = Arc::new(move |event, payload| {
        let _ = app_for_events.emit(event, payload);
    });
    let context = LoadContext {
        run_id: run_id.clone(),
        server_id: server_id.clone(),
        server_name: prepared.server_name,
        profile,
        operations: prepared.operations,
        config: prepared.config,
        oauth: Some(state.oauth.clone()),
        event_sink: Some(event_sink),
    };
    let run_id_for_task = run_id.clone();
    tauri::async_runtime::spawn(async move {
        let run_id = run_id_for_task;
        let outcome = run_engine(context, Arc::clone(&control)).await;
        let summary = outcome.summary.clone();
        let series = outcome.series.clone();
        let errors = outcome.errors.clone();
        let status = outcome.status;
        let stop_reason = outcome.stop_reason.clone();
        let run_id_for_db = run_id.clone();
        let stop_reason_for_db = stop_reason.clone();
        if let Err(error) = db
            .with_write(move |conn| {
                load_repo::finish_run(
                    conn,
                    &run_id_for_db,
                    status,
                    stop_reason_for_db.as_deref(),
                    &summary,
                    &series,
                    &errors,
                    outcome.finished_at,
                )
            })
            .await
        {
            tracing::warn!(run_id = %run_id, error = %error, "压测运行落库失败");
        }
        let _ = app_for_task.emit(
            "loadtest://run-finished",
            json!({
                "run_id": run_id.clone(),
                "status": status,
                "stop_reason": stop_reason,
            }),
        );
        let state = app_for_task.state::<AppState>();
        {
            let mut guard = state.load_run.lock().await;
            if guard
                .as_ref()
                .is_some_and(|control| control.run_id == run_id)
            {
                *guard = None;
            }
        }
        clear_run_slot(&state.run_slot, RunKind::Load, &run_id);
    });

    Ok(run_id)
}

#[tauri::command]
pub async fn cancel_load_test(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    let control = {
        let guard = state.load_run.lock().await;
        guard
            .as_ref()
            .filter(|control| control.run_id == run_id)
            .cloned()
    };
    let Some(control) = control else {
        return Ok(false);
    };
    control.request_cancel("user");
    Ok(true)
}

#[tauri::command]
pub async fn get_active_load_test(state: State<'_, AppState>) -> Result<Option<ActiveLoadRun>, String> {
    let control = {
        let guard = state.load_run.lock().await;
        guard.as_ref().cloned()
    };
    let Some(control) = control else {
        return Ok(None);
    };
    let run = state
        .db
        .with_read_blocking(|conn| load_repo::get_run(conn, &control.run_id))
        .map_err(|e| e.to_string())?;
    let Some(run) = run else {
        return Ok(None);
    };
    Ok(Some(ActiveLoadRun {
        run_id: run.id,
        server_id: run.server_id,
        suite_name: run.suite_name,
        profile_name: run.profile_name,
        started_at: run.started_at,
        latest: control.latest().clone(),
    }))
}

#[tauri::command]
pub async fn list_load_runs(
    state: State<'_, AppState>,
    server_id: Option<String>,
    offset: u64,
    limit: u32,
) -> Result<Paged<LoadRunSummary>, String> {
    let limit = limit.clamp(1, 200);
    let (items, total) = state
        .db
        .with_read_blocking(|conn| load_repo::list_runs(conn, server_id.as_deref(), offset, limit))
        .map_err(|e| e.to_string())?;
    Ok(Paged {
        items,
        total,
        offset,
        limit: limit as u64,
    })
}

#[tauri::command]
pub async fn get_load_run(state: State<'_, AppState>, run_id: String) -> Result<LoadRun, String> {
    state
        .db
        .with_read_blocking(|conn| load_repo::get_run(conn, &run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "压测运行不存在".to_string())
}

#[tauri::command]
pub async fn export_load_run(
    state: State<'_, AppState>,
    run_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let run = state
        .db
        .with_read_blocking(|conn| load_repo::get_run(conn, &run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "压测运行不存在".to_string())?;
    let json = load_repo::export_json(&run).map_err(|e| e.to_string())?;
    let target = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => state.data_dir.join("load-reports").join(format!("load-{run_id}.json")),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&target, json).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

async fn prepare_load(
    state: &AppState,
    server_id: &str,
    suite_id: &str,
    profile: &LoadProfile,
    refresh_capabilities: bool,
) -> Result<PreparedLoadData, String> {
    let handle = state
        .manager
        .handle(server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let (protocol_version, lifecycle) = match handle.state().await {
        ConnectionState::Ready { handshake } => (
            handshake.protocol_version,
            format!("{:?}", handshake.lifecycle).to_lowercase(),
        ),
        _ => return Err("服务器未就绪，请先连接".to_string()),
    };
    let capabilities = match handle.capabilities(refresh_capabilities).await {
        Ok(snapshot) => snapshot,
        Err(first) if refresh_capabilities => handle
            .capabilities(false)
            .await
            .map_err(|_| first.message())?,
        Err(error) => return Err(error.message()),
    };
    let config = crate::commands::server::load_resolved_config(state, server_id)
        .map_err(|error| error.message)?;
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    if suite.server_id != server_id {
        return Err("套件不属于所选服务器".to_string());
    }
    validate_profile(profile, &config.transport, config.http.as_ref().is_some_and(|http| http.oauth.is_some()))?;

    let mut operations = Vec::new();
    let mut preview = Vec::new();
    for operation in &profile.operations {
        let case = state
            .db
            .with_read_blocking(|conn| test_repo::get_case(conn, &operation.case_id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("用例不存在：{}", operation.case_id))?;
        if case.suite_id != suite.id {
            return Err(format!("用例不属于该套件：{}", operation.case_id));
        }
        if !case.enabled {
            return Err(format!("用例已禁用：{}", case.name));
        }
        let risk = effective_risk(&case, &capabilities);
        let mut snapshot = snapshot_operation(&case, operation.weight);
        snapshot.risk = risk.clone();
        let mut preview_item = preview_operation(
            &snapshot,
            risk,
            profile.allow_write,
            profile.allow_unknown,
        );
        if !target_exists(&case, &capabilities) {
            preview_item.allowed = false;
            preview_item.reason = Some("target_missing".to_string());
        }
        preview.push(preview_item);
        operations.push(snapshot);
    }
    let risk_summary = summarize_risks(&operations);
    let production = config.tags.iter().any(|tag| {
        matches!(
            tag.to_ascii_lowercase().as_str(),
            "prod" | "production" | "生产"
        )
    });
    Ok(PreparedLoadData {
        server_name: config.name.clone(),
        config,
        suite_id: suite.id,
        suite_name: suite.name,
        operations,
        preview,
        risk_summary,
        production,
        protocol_version,
        lifecycle,
        transport: handle.transport_kind().await.as_str().to_string(),
        capability_hash: crate::testing::contract::capability_hash(&capabilities),
        capability_stale: capabilities.stale,
        tool_count: capabilities.tools.len() as u64,
        resource_count: capabilities.resources.len() as u64,
        template_count: capabilities.resource_templates.len() as u64,
        prompt_count: capabilities.prompts.len() as u64,
    })
}

fn reason_text(reason: Option<&str>) -> String {
    match reason {
        Some("write_not_allowed") => "未授权写操作".to_string(),
        Some("unknown_not_allowed") => "未授权未知风险".to_string(),
        Some("destructive_not_allowed") => "破坏性操作禁止压测".to_string(),
        Some("target_missing") => "当前服务器缺少该接口".to_string(),
        Some(other) => other.to_string(),
        None => "未知原因".to_string(),
    }
}
