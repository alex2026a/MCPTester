//! v0.2.1 测试命令：套件/用例/契约快照/导入导出。

use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::commands::AppState;
use crate::mcp::models::{CallMethod, CallOrigin, CallRecord, CallStatus, CapabilitySnapshot};
use crate::storage::{call_repo, settings_repo, test_repo};
use crate::testing::report;
use crate::testing::contract::{
    build_snapshot, capability_hash as compute_capability_hash,
    diff_contract as contract_diff,
};
use crate::testing::generator;
use crate::testing::history;
use crate::testing::models::{
    ActiveTestRun, AssertionKind, AssertionSeverity, ComparisonItem, ContractDiff, ContractSnapshot,
    CoverageDelta, GenerationOptions, GenerationPreview, ImportTestCasesReport, ImportTestIssue,
    Paged, RunComparison, RunControl, RunOptions, TestAssertion, TestCase, TestCaseDraft,
    TestExpect, TestMethod, TestOutcome, TestRunCaseDetail, TestRunCaseFilter,
    TestRunCaseStatus, TestRunCaseSummary, TestRunDetail, TestRunFilter, TestRunSummary,
    TestSource, TestSuite, TestSuiteDraft, TestSuiteSummary, TestSuiteUpdateDraft,
};
use crate::testing::runner;

/// 列表（可按服务器过滤）
#[tauri::command]
pub async fn list_test_suites(
    state: State<'_, AppState>,
    server_id: Option<String>,
) -> Result<Vec<TestSuiteSummary>, String> {
    state
        .db
        .with_read_blocking(|conn| test_repo::list_suites(conn, server_id.as_deref()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_test_suite(
    state: State<'_, AppState>,
    id: String,
) -> Result<TestSuite, String> {
    state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())
}

#[tauri::command]
pub async fn create_test_suite(
    state: State<'_, AppState>,
    draft: TestSuiteDraft,
) -> Result<TestSuite, String> {
    if draft.name.trim().is_empty() {
        return Err("套件名称不能为空".to_string());
    }
    if draft.server_id.trim().is_empty() {
        return Err("套件必须绑定服务器".to_string());
    }
    state
        .db
        .with_write_blocking(|conn| test_repo::create_suite(conn, &draft))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_test_suite(
    state: State<'_, AppState>,
    id: String,
    draft: TestSuiteUpdateDraft,
) -> Result<TestSuite, String> {
    if draft.name.trim().is_empty() {
        return Err("套件名称不能为空".to_string());
    }
    state
        .db
        .with_write_blocking(|conn| test_repo::update_suite(conn, &id, &draft))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rebind_test_suite(
    state: State<'_, AppState>,
    id: String,
    server_id: String,
) -> Result<TestSuite, String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::rebind_suite(conn, &id, &server_id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_test_suite(state: State<'_, AppState>, id: String) -> Result<u64, String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::delete_suite(conn, &id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_test_cases(
    state: State<'_, AppState>,
    suite_id: String,
) -> Result<Vec<TestCase>, String> {
    state
        .db
        .with_read_blocking(|conn| test_repo::list_cases(conn, &suite_id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_test_cases(
    state: State<'_, AppState>,
    suite_id: String,
    cases: Vec<TestCaseDraft>,
) -> Result<Vec<TestCase>, String> {
    if cases.is_empty() {
        return Err("没有要创建的用例".to_string());
    }
    for case in &cases {
        validate_draft(case)?;
    }
    state
        .db
        .with_write_blocking(|conn| {
            let tx = conn.transaction()?;
            let inserted = test_repo::insert_cases(&tx, &suite_id, &cases)?;
            tx.commit()?;
            Ok(inserted)
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_test_case(
    state: State<'_, AppState>,
    id: String,
    draft: TestCaseDraft,
) -> Result<TestCase, String> {
    validate_draft(&draft)?;
    state
        .db
        .with_write_blocking(|conn| test_repo::update_case(conn, &id, &draft))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_test_cases(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<u64, String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::delete_cases(conn, &ids))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn duplicate_test_case(
    state: State<'_, AppState>,
    id: String,
) -> Result<TestCase, String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::duplicate_case(conn, &id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reorder_test_cases(
    state: State<'_, AppState>,
    suite_id: String,
    ids: Vec<String>,
) -> Result<(), String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::reorder_cases(conn, &suite_id, &ids))
        .map_err(|e| e.to_string())
}

/// 生成用例预览（不落库）。
#[tauri::command]
pub async fn generate_test_cases(
    state: State<'_, AppState>,
    server_id: String,
    options: GenerationOptions,
) -> Result<GenerationPreview, String> {
    let handle = state
        .manager
        .handle(&server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let capabilities = handle
        .capabilities(true)
        .await
        .map_err(|e| e.message())?;
    let samples = if options.use_history {
        state
            .db
            .with_read_blocking(|conn| history::load_samples(conn, &server_id, 200))
            .map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    Ok(generator::generate(&capabilities, &samples, &options))
}

/// 保存生成预览：原子写入用例 + 能力快照/指纹。
#[tauri::command]
pub async fn save_generated_cases(
    state: State<'_, AppState>,
    suite_id: String,
    cases: Vec<TestCaseDraft>,
    capability_hash: String,
    capability_snapshot: Value,
) -> Result<Vec<TestCase>, String> {
    if cases.is_empty() {
        return Err("没有要保存的用例".to_string());
    }
    for case in &cases {
        validate_draft(case)?;
    }
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    let snapshot: CapabilitySnapshot = serde_json::from_value(capability_snapshot.clone())
        .map_err(|e| format!("能力快照不合法：{e}"))?;
    if snapshot.server_id != suite.server_id {
        return Err("能力快照与套件不属于同一服务器".to_string());
    }
    if compute_capability_hash(&snapshot) != capability_hash {
        return Err("能力指纹不匹配，预览可能已过期，请重新生成".to_string());
    }
    state
        .db
        .with_write_blocking(|conn| {
            let tx = conn.transaction()?;
            test_repo::set_suite_capability(&tx, &suite_id, &capability_hash, &capability_snapshot)?;
            let inserted = test_repo::insert_cases(&tx, &suite_id, &cases)?;
            tx.commit()?;
            Ok(inserted)
        })
        .map_err(|e| e.to_string())
}

/// 运行套件（后台执行，返回 run_id）。
#[tauri::command]
pub async fn run_test_suite(
    app: AppHandle,
    state: State<'_, AppState>,
    suite_id: String,
    options: RunOptions,
) -> Result<String, String> {
    ensure_no_active_run(&state).await?;
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    let mut cases = state
        .db
        .with_read_blocking(|conn| test_repo::list_cases(conn, &suite_id))
        .map_err(|e| e.to_string())?;
    cases.retain(|case| case.enabled);
    if let Some(ids) = &options.case_ids {
        cases.retain(|case| ids.contains(&case.id));
    }
    if cases.is_empty() {
        return Err("没有可运行的启用用例".to_string());
    }
    let settings = state
        .db
        .with_read_blocking(settings_repo::get)
        .map_err(|e| e.to_string())?;
    let prepared = runner::prepare_run(
        &state.db,
        &state.manager,
        &suite.server_id,
        Some(suite.id.clone()),
        suite.name.clone(),
        cases.clone(),
        options,
    )
    .await?;
    start_background(
        app,
        &state,
        prepared,
        cases,
        settings.request_timeout_secs,
    )
    .await
}

/// 运行指定用例（必须属于同一服务器）。
#[tauri::command]
pub async fn run_test_cases(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: String,
    case_ids: Vec<String>,
    options: RunOptions,
) -> Result<String, String> {
    ensure_no_active_run(&state).await?;
    if case_ids.is_empty() {
        return Err("没有选中的用例".to_string());
    }
    let mut cases = Vec::new();
    let mut suite_ids = std::collections::HashSet::new();
    for id in &case_ids {
        let case = state
            .db
            .with_read_blocking(|conn| test_repo::get_case(conn, id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("用例不存在：{id}"))?;
        let suite = state
            .db
            .with_read_blocking(|conn| test_repo::get_suite(conn, &case.suite_id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "用例所属套件不存在".to_string())?;
        if suite.server_id != server_id {
            return Err("选中的用例不属于同一服务器".to_string());
        }
        suite_ids.insert(suite.id.clone());
        cases.push(case);
    }
    cases.sort_by(|left, right| {
        left.suite_id
            .cmp(&right.suite_id)
            .then(left.order_index.cmp(&right.order_index))
            .then(left.id.cmp(&right.id))
    });
    let (suite_id, suite_name) = if suite_ids.len() == 1 {
        let id = suite_ids.into_iter().next().unwrap();
        let suite = state
            .db
            .with_read_blocking(|conn| test_repo::get_suite(conn, &id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "套件不存在".to_string())?;
        (Some(id), suite.name)
    } else {
        (None, "多套件".to_string())
    };
    let settings = state
        .db
        .with_read_blocking(settings_repo::get)
        .map_err(|e| e.to_string())?;
    let prepared = runner::prepare_run(
        &state.db,
        &state.manager,
        &server_id,
        suite_id,
        suite_name,
        cases.clone(),
        options,
    )
    .await?;
    start_background(
        app,
        &state,
        prepared,
        cases,
        settings.request_timeout_secs,
    )
    .await
}

async fn start_background(
    app: AppHandle,
    state: &AppState,
    prepared: runner::PreparedRun,
    cases: Vec<TestCase>,
    default_timeout_secs: u64,
) -> Result<String, String> {
    let run_id = prepared.run.id.clone();
    let server_id = prepared.run.server_id.clone();
    let control = RunControl::new(run_id.clone(), server_id);
    let control_guard = state.test_run.clone();
    {
        let mut guard = control_guard.lock().await;
        if guard.is_some() {
            return Err("已有测试正在运行".to_string());
        }
        *guard = Some(control.clone());
    }
    if let Err(error) = state
        .db
        .with_write_blocking(|conn| test_repo::create_run(conn, &prepared.run))
    {
        let mut guard = control_guard.lock().await;
        *guard = None;
        return Err(format!("创建运行记录失败：{error}"));
    }
    runner::spawn(
        app,
        state.db.clone(),
        state.manager.clone(),
        state.data_dir.clone(),
        prepared,
        cases,
        control,
        default_timeout_secs,
    );
    Ok(run_id)
}

async fn ensure_no_active_run(state: &AppState) -> Result<(), String> {
    if state.test_run.lock().await.is_some() {
        return Err("已有测试正在运行，请先取消".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn cancel_test_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<bool, String> {
    let control = {
        let guard = state.test_run.lock().await;
        guard
            .as_ref()
            .filter(|control| control.run_id == run_id)
            .cloned()
    };
    let Some(control) = control else {
        return Ok(false);
    };
    control.request_cancel();
    if let Some(call_id) = control.current_call()
        && let Some(handle) = state.manager.handle(&control.server_id).await
    {
        let _ = handle.cancel_call(&call_id).await;
    }
    Ok(true)
}

#[tauri::command]
pub async fn get_active_test_run(
    state: State<'_, AppState>,
) -> Result<Option<ActiveTestRun>, String> {
    let control = state.test_run.lock().await.clone();
    let Some(control) = control else {
        return Ok(None);
    };
    let run = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &control.run_id))
        .map_err(|e| e.to_string())?;
    let Some(run) = run else {
        return Ok(None);
    };
    Ok(Some(ActiveTestRun {
        run_id: run.id,
        server_id: run.server_id,
        total: run.summary.total,
        completed: run.summary.passed
            + run.summary.failed
            + run.summary.warning
            + run.summary.skipped
            + run.summary.error
            + run.summary.canceled,
        started_at: run.started_at,
    }))
}

#[tauri::command]
pub async fn list_test_runs(
    state: State<'_, AppState>,
    filter: TestRunFilter,
    offset: u64,
    limit: u64,
) -> Result<Paged<TestRunSummary>, String> {
    let limit = limit.clamp(1, 200);
    state
        .db
        .with_read_blocking(|conn| test_repo::list_runs(conn, &filter, offset, limit))
        .map(|(items, total)| Paged {
            items,
            total,
            offset,
            limit,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_test_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<TestRunDetail, String> {
    let run = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "运行不存在".to_string())?;
    Ok(TestRunDetail {
        coverage: run.summary.coverage.clone(),
        run,
    })
}

#[tauri::command]
pub async fn list_test_run_cases(
    state: State<'_, AppState>,
    run_id: String,
    filter: TestRunCaseFilter,
    offset: u64,
    limit: u64,
) -> Result<Paged<TestRunCaseSummary>, String> {
    let limit = limit.clamp(1, 500);
    state
        .db
        .with_read_blocking(|conn| {
            test_repo::list_run_cases(conn, &run_id, &filter, offset, limit)
        })
        .map(|(items, total)| Paged {
            items,
            total,
            offset,
            limit,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_test_run_case(
    state: State<'_, AppState>,
    run_case_id: String,
) -> Result<TestRunCaseDetail, String> {
    state
        .db
        .with_read_blocking(|conn| test_repo::get_run_case(conn, &run_case_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "运行用例不存在".to_string())
}

#[tauri::command]
pub async fn set_test_baseline(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<(), String> {
    state
        .db
        .with_write_blocking(|conn| test_repo::set_baseline(conn, &run_id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compare_test_runs(
    state: State<'_, AppState>,
    base_run_id: String,
    target_run_id: String,
) -> Result<RunComparison, String> {
    let base = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &base_run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "基线运行不存在".to_string())?;
    let target = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &target_run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "目标运行不存在".to_string())?;
    let base_cases = state
        .db
        .with_read_blocking(|conn| test_repo::get_all_run_cases(conn, &base_run_id))
        .map_err(|e| e.to_string())?;
    let target_cases = state
        .db
        .with_read_blocking(|conn| test_repo::get_all_run_cases(conn, &target_run_id))
        .map_err(|e| e.to_string())?;
    Ok(build_comparison(&base, &target, &base_cases, &target_cases))
}

#[tauri::command]
pub async fn rerun_failed_test_cases(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    options: RunOptions,
) -> Result<String, String> {
    ensure_no_active_run(&state).await?;
    let run = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "运行不存在".to_string())?;
    let cases = state
        .db
        .with_read_blocking(|conn| test_repo::get_all_run_cases(conn, &run_id))
        .map_err(|e| e.to_string())?;
    let mut rerun = Vec::new();
    for case in cases {
        if !matches!(
            case.summary.status,
            TestRunCaseStatus::Failed | TestRunCaseStatus::Error
        ) {
            continue;
        }
        let parsed: TestCase = serde_json::from_value(case.case_snapshot)
            .map_err(|e| format!("用例快照无法恢复：{e}"))?;
        rerun.push(parsed);
    }
    if rerun.is_empty() {
        return Err("没有失败用例可重跑".to_string());
    }
    let settings = state
        .db
        .with_read_blocking(settings_repo::get)
        .map_err(|e| e.to_string())?;
    let prepared = runner::prepare_run(
        &state.db,
        &state.manager,
        &run.server_id,
        run.suite_id.clone(),
        run.suite_name.clone(),
        rerun.clone(),
        options,
    )
    .await?;
    start_background(
        app,
        &state,
        prepared,
        rerun,
        settings.request_timeout_secs,
    )
    .await
}

#[tauri::command]
pub async fn export_test_report(
    state: State<'_, AppState>,
    run_id: String,
    format: String,
    path: Option<String>,
    include_full: bool,
    include_sensitive: bool,
) -> Result<String, String> {
    let run = state
        .db
        .with_read_blocking(|conn| test_repo::get_run(conn, &run_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "运行不存在".to_string())?;
    let cases = state
        .db
        .with_read_blocking(|conn| test_repo::get_all_run_cases(conn, &run_id))
        .map_err(|e| e.to_string())?;
    let report = report::build(run.clone(), run.summary.coverage.clone(), cases);
    let content = match format.as_str() {
        "html" => report::to_html(&report, include_sensitive),
        "json" => report::to_json(&report, include_sensitive),
        "junit" => report::to_junit(&report),
        "md" | "markdown" => report::to_markdown(&report),
        other => return Err(format!("不支持的报告格式：{other}")),
    };
    if let Some(path) = path {
        report::write(std::path::Path::new(&path), &content)
            .map_err(|e| format!("写入报告失败：{e}"))?;
        return Ok(path);
    }
    let _ = include_full;
    Ok(content)
}

fn build_comparison(
    base: &crate::testing::models::TestRun,
    target: &crate::testing::models::TestRun,
    base_cases: &[TestRunCaseDetail],
    target_cases: &[TestRunCaseDetail],
) -> RunComparison {
    let mut base_map = std::collections::HashMap::new();
    for case in base_cases {
        base_map.insert(comparison_key(&case.summary), case);
    }
    let mut target_map = std::collections::HashMap::new();
    for case in target_cases {
        target_map.insert(comparison_key(&case.summary), case);
    }
    let mut new_failures = Vec::new();
    let mut fixed = Vec::new();
    let mut still_failing = Vec::new();
    let mut warning_changes = Vec::new();
    let mut error_cases = Vec::new();
    let mut new_cases = Vec::new();
    let mut removed_cases = Vec::new();

    for (key, target_case) in &target_map {
        let base_case = base_map.get(key);
        let base_case = base_case.copied();
        let item = comparison_item(key, base_case, Some(*target_case));
        match (base_case.map(|case| &case.summary.status), &target_case.summary.status) {
            (None, _) => new_cases.push(item),
            (Some(TestRunCaseStatus::Failed), TestRunCaseStatus::Failed) => {
                still_failing.push(item)
            }
            (Some(TestRunCaseStatus::Failed), TestRunCaseStatus::Passed) => fixed.push(item),
            (Some(TestRunCaseStatus::Failed), _) => new_failures.push(item),
            (Some(_), TestRunCaseStatus::Failed) => new_failures.push(item),
            (Some(TestRunCaseStatus::Warning), _)
            | (Some(_), TestRunCaseStatus::Warning) => warning_changes.push(item),
            _ => {}
        }
        if target_case.summary.status == TestRunCaseStatus::Error {
            error_cases.push(comparison_item(key, base_case, Some(*target_case)));
        }
    }
    for (key, base_case) in &base_map {
        if !target_map.contains_key(key) {
            removed_cases.push(comparison_item(key, Some(*base_case), None));
        }
    }

    let base_pass = base.summary.effective_pass_rate;
    let target_pass = target.summary.effective_pass_rate;
    RunComparison {
        base_run_id: base.id.clone(),
        target_run_id: target.id.clone(),
        new_failures,
        fixed,
        still_failing,
        warning_changes,
        error_cases,
        new_cases,
        removed_cases,
        duration_delta_ms: target.summary.duration_ms as i64 - base.summary.duration_ms as i64,
        pass_rate_delta: target.summary.pass_rate - base.summary.pass_rate,
        effective_pass_rate_delta: target_pass - base_pass,
        coverage_delta: CoverageDelta {
            tools: ratio_delta(base.summary.coverage.tools_covered, base.summary.coverage.tools_total, target.summary.coverage.tools_covered, target.summary.coverage.tools_total),
            required_params: ratio_delta(base.summary.coverage.required_params_covered, base.summary.coverage.required_params_total, target.summary.coverage.required_params_covered, target.summary.coverage.required_params_total),
            negative_categories: ratio_delta(
                base.summary.coverage.negative_categories.iter().filter(|item| item.covered).count() as u64,
                base.summary.coverage.negative_categories.len() as u64,
                target.summary.coverage.negative_categories.iter().filter(|item| item.covered).count() as u64,
                target.summary.coverage.negative_categories.len() as u64,
            ),
        },
        contract_changed: base.manifest.capability_hash != target.manifest.capability_hash,
    }
}

fn comparison_key(case: &TestRunCaseSummary) -> String {
    case.case_id
        .clone()
        .unwrap_or_else(|| format!("{}|{}|{}", case.method.as_str(), case.target, case.case_name))
}

fn comparison_item(
    key: &str,
    base: Option<&TestRunCaseDetail>,
    target: Option<&TestRunCaseDetail>,
) -> ComparisonItem {
    let summary = target.map(|case| &case.summary).or_else(|| base.map(|case| &case.summary));
    ComparisonItem {
        key: key.to_string(),
        name: summary.map(|case| case.case_name.clone()).unwrap_or_default(),
        base_status: base.map(|case| case.summary.status.clone()),
        target_status: target.map(|case| case.summary.status.clone()),
        failure_kind: target.and_then(|case| case.summary.failure_kind.clone()),
    }
}

fn ratio_delta(base_covered: u64, base_total: u64, target_covered: u64, target_total: u64) -> f64 {
    let base = if base_total > 0 {
        base_covered as f64 / base_total as f64
    } else {
        0.0
    };
    let target = if target_total > 0 {
        target_covered as f64 / target_total as f64
    } else {
        0.0
    };
    target - base
}

/// 当前能力快照（生成前强制刷新）。
#[tauri::command]
pub async fn get_contract_snapshot(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<ContractSnapshot, String> {
    let handle = state
        .manager
        .handle(&server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let snapshot = handle
        .capabilities(true)
        .await
        .map_err(|e| e.message())?;
    Ok(build_snapshot(&snapshot))
}

/// 对比套件保存的契约与当前服务器。
#[tauri::command]
pub async fn diff_contract(
    state: State<'_, AppState>,
    suite_id: String,
) -> Result<ContractDiff, String> {
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    let old = suite
        .capability_snapshot
        .ok_or_else(|| "套件尚未保存契约快照，请先生成用例".to_string())?;
    let handle = state
        .manager
        .handle(&suite.server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let current = handle
        .capabilities(true)
        .await
        .map_err(|e| e.message())?;
    Ok(contract_diff(&old, &current))
}

/// 从历史调用保存为用例（M1：最小参数映射，不做快照断言）。
#[tauri::command]
pub async fn save_call_as_case(
    state: State<'_, AppState>,
    record_id: String,
    suite_id: String,
    name: Option<String>,
) -> Result<TestCase, String> {
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    let record = state
        .db
        .with_read_blocking(|conn| call_repo::get(conn, &record_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "调用记录不存在".to_string())?;
    if record.method == CallMethod::Probe {
        return Err("探测记录不支持保存为测试用例".to_string());
    }
    if record.server_id != suite.server_id {
        return Err("调用记录与套件不属于同一服务器".to_string());
    }
    let draft = case_from_record(&record, name)?;
    state
        .db
        .with_write_blocking(|conn| {
            let tx = conn.transaction()?;
            let inserted = test_repo::insert_cases(&tx, &suite_id, &[draft])?;
            tx.commit()?;
            inserted
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("保存用例失败"))
        })
        .map_err(|e| e.to_string())
}

/// 导出套件 JSON（schema_version=1）。
#[tauri::command]
pub async fn export_test_suite(
    state: State<'_, AppState>,
    suite_id: String,
    path: Option<String>,
) -> Result<String, String> {
    let suite = state
        .db
        .with_read_blocking(|conn| test_repo::get_suite(conn, &suite_id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "套件不存在".to_string())?;
    let cases = state
        .db
        .with_read_blocking(|conn| test_repo::list_cases(conn, &suite_id))
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "kind": "mcptester.test-suite",
        "schema_version": 1,
        "exported_by": format!("MCP Tester v{}", env!("CARGO_PKG_VERSION")),
        "suite": {
            "name": suite.name,
            "description": suite.description,
            "server_id": suite.server_id,
            "tags": suite.tags,
            "capability_hash": suite.capability_hash,
            "capability_snapshot": suite.capability_snapshot,
        },
        "cases": cases,
    }))
    .map_err(|e| e.to_string())?;
    if let Some(path) = path {
        std::fs::write(&path, &json).map_err(|e| format!("写入失败：{e}"))?;
    }
    Ok(json)
}

#[derive(Deserialize)]
struct ImportSuitePayload {
    #[serde(default)]
    cases: Vec<Value>,
}

/// 导入套件/用例 JSON；LLM 导入使用 source=llm。
#[tauri::command]
pub async fn import_test_cases(
    state: State<'_, AppState>,
    suite_id: String,
    json: String,
    source: Option<String>,
) -> Result<ImportTestCasesReport, String> {
    import_cases(&state, &suite_id, &json, source.as_deref())
}

/// LLM 用例导入：等价于 import_test_cases(source=llm)。
#[tauri::command]
pub async fn import_llm_test_cases(
    state: State<'_, AppState>,
    suite_id: String,
    json: String,
) -> Result<ImportTestCasesReport, String> {
    import_cases(&state, &suite_id, &json, Some("llm"))
}

#[tauri::command]
pub async fn export_llm_prompt(
    state: State<'_, AppState>,
    server_id: String,
    options: GenerationOptions,
) -> Result<String, String> {
    let handle = state
        .manager
        .handle(&server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let snapshot = handle
        .capabilities(true)
        .await
        .map_err(|e| e.message())?;
    Ok(crate::testing::llm::build_prompt(&snapshot, &options))
}

fn import_cases(
    state: &AppState,
    suite_id: &str,
    json: &str,
    source: Option<&str>,
) -> Result<ImportTestCasesReport, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let cases_value = match value {
        Value::Array(items) => items,
        Value::Object(_) => {
            let payload: ImportSuitePayload =
                serde_json::from_value(value).map_err(|e| format!("结构不合法：{e}"))?;
            payload.cases
        }
        _ => return Err("JSON 必须是数组或包含 cases 的对象".to_string()),
    };
    let source = match source {
        Some("llm") => TestSource::Llm,
        Some("history") => TestSource::History,
        Some("schema") => TestSource::Schema,
        _ => TestSource::Import,
    };
    let mut drafts = Vec::new();
    let mut failed = Vec::new();
    let mut skipped = 0u32;
    for (index, item) in cases_value.into_iter().enumerate() {
        let method = item
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("tools_call")
            .to_string();
        let target = item
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("导入用例 {index}"));
        match serde_json::from_value::<TestCaseDraft>(item) {
            Ok(mut draft) => {
                if target.trim().is_empty() {
                    failed.push(ImportTestIssue {
                        index: index as u32,
                        name: Some(name),
                        reason: "target 不能为空".to_string(),
                    });
                    continue;
                }
                if draft.method != TestMethod::ToolsCall
                    && draft.method != TestMethod::ResourcesRead
                    && draft.method != TestMethod::PromptsGet
                {
                    failed.push(ImportTestIssue {
                        index: index as u32,
                        name: Some(name),
                        reason: format!("不支持的方法：{method}"),
                    });
                    continue;
                }
                draft.source = source.clone();
                draft.enabled = false;
                if matches!(source, TestSource::Llm) {
                    draft.tags.push("llm".to_string());
                    draft.tags.push("needs-review".to_string());
                }
                if let Err(reason) = validate_draft(&draft) {
                    failed.push(ImportTestIssue {
                        index: index as u32,
                        name: Some(name),
                        reason,
                    });
                } else {
                    drafts.push(draft);
                }
            }
            Err(error) => failed.push(ImportTestIssue {
                index: index as u32,
                name: Some(name),
                reason: error.to_string(),
            }),
        }
    }
    if drafts.is_empty() {
        return Ok(ImportTestCasesReport {
            added: 0,
            skipped,
            failed,
        });
    }
    let added = state
        .db
        .with_write_blocking(|conn| {
            let tx = conn.transaction()?;
            let inserted = test_repo::insert_cases(&tx, suite_id, &drafts)?;
            tx.commit()?;
            Ok(inserted.len() as u32)
        })
        .map_err(|e| e.to_string())?;
    skipped += 0;
    Ok(ImportTestCasesReport {
        added,
        skipped,
        failed,
    })
}

fn validate_draft(draft: &TestCaseDraft) -> Result<(), String> {
    if draft.name.trim().is_empty() {
        return Err("用例名称不能为空".to_string());
    }
    if draft.target.trim().is_empty() {
        return Err("target 不能为空".to_string());
    }
    if !draft.arguments.is_object() && !draft.arguments.is_null() {
        return Err("arguments 必须是 JSON 对象".to_string());
    }
    Ok(())
}

fn case_from_record(record: &CallRecord, name: Option<String>) -> Result<TestCaseDraft, String> {
    let (method, target, arguments) = match record.method.clone() {
        CallMethod::ToolsCall => {
            let target = record
                .params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "记录缺少工具名".to_string())?;
            let arguments = record
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            (TestMethod::ToolsCall, target.to_string(), arguments)
        }
        CallMethod::ResourcesRead => {
            let target = record
                .params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| "记录缺少资源 URI".to_string())?;
            (
                TestMethod::ResourcesRead,
                target.to_string(),
                Value::Object(Default::default()),
            )
        }
        CallMethod::PromptsGet => {
            let target = record
                .params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "记录缺少提示词名".to_string())?;
            let arguments = record
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            (TestMethod::PromptsGet, target.to_string(), arguments)
        }
        CallMethod::Probe => return Err("探测记录不支持保存为测试用例".to_string()),
    };
    let ok = record.status == CallStatus::Ok;
    let expect = if ok {
        TestExpect {
            outcome: TestOutcome::Success,
            ..Default::default()
        }
    } else {
        TestExpect {
            outcome: TestOutcome::Any,
            ..Default::default()
        }
    };
    let assertions = if ok {
        vec![TestAssertion {
            id: uuid::Uuid::new_v4().to_string(),
            kind: AssertionKind::McpShape,
            severity: AssertionSeverity::Error,
            enabled: true,
            target: None,
            path: None,
            op: None,
            expected: None,
            value_ms: None,
            message: Some("MCP 结果基本形状".to_string()),
            ignore_paths: Vec::new(),
            schema: None,
            schema_ref: None,
        }]
    } else {
        Vec::new()
    };
    Ok(TestCaseDraft {
        name: name.unwrap_or_else(|| format!("{target} · 来自历史")),
        description: format!("由调用记录 {} 保存", record.id),
        method,
        target,
        arguments,
        expect,
        assertions,
        tags: vec!["from-history".to_string()],
        priority: crate::testing::models::TestPriority::P1,
        risk: crate::testing::models::TestRisk::Unknown,
        source: TestSource::History,
        generation_note: Some(format!("历史调用 {}（origin={}）", record.id, record.origin.as_str())),
        enabled: true,
        timeout_secs: None,
        retry_count: 0,
    })
}

/// 测试命令内部使用：确保 origin 枚举可序列化。
#[allow(dead_code)]
fn _origin_smoke(origin: CallOrigin) -> String {
    serde_json::to_string(&origin).unwrap_or_default()
}
