//! 单步测试运行器：运行锁、取消、重试、证据采集与增量落库。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::commands::AppState;
use crate::mcp::connection::{CallOptions, ConnectionHandle};
use crate::mcp::error::CallError;
use crate::mcp::models::{CallOrigin, CallRecord, CallStatus, CapabilitySnapshot};
use crate::storage::{Database, call_repo, server_repo, test_repo};

use super::assertions;
use super::contract::{capability_hash, suite_hash};
use super::coverage;
use super::generator;
use super::models::{
    AssertionResult, Evidence, FailureKind, RunControl, RunManifest, RunOptions, RunSummary,
    TestCase, TestMethod, TestRisk, TestRun, TestRunCaseStatus, TestRunStatus,
};

/// 运行准备结果。
pub struct PreparedRun {
    pub run: TestRun,
    pub capabilities: CapabilitySnapshot,
    pub server_tags: Vec<String>,
}

/// 准备运行：检查连接、采集契约、构建 manifest。
#[allow(clippy::too_many_arguments)]
pub async fn prepare_run(
    db: &Database,
    manager: &Arc<crate::mcp::ConnectionManager>,
    server_id: &str,
    suite_id: Option<String>,
    suite_name: String,
    cases: Vec<TestCase>,
    options: RunOptions,
) -> Result<PreparedRun, String> {
    let handle = manager
        .handle(server_id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let state = handle.state().await;
    let handshake = match state {
        crate::mcp::models::ConnectionState::Ready { handshake } => handshake,
        _ => return Err("服务器未连接，请先连接".to_string()),
    };
    let capabilities = match handle.capabilities(true).await {
        Ok(snapshot) => snapshot,
        Err(error) => handle
            .capabilities(false)
            .await
            .map_err(|_| error.message())?,
    };
    let server_id_owned = server_id.to_string();
    let server = db
        .with_read(move |conn| server_repo::get(conn, &server_id_owned))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "服务器不存在".to_string())?;
    let transport = handle.transport_kind().await;
    let started_at = Utc::now();
    let manifest = RunManifest {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        server_id: server_id.to_string(),
        server_name: server.name.clone(),
        protocol_version: handshake.protocol_version.clone(),
        lifecycle: format!("{:?}", handshake.lifecycle).to_lowercase(),
        transport: transport.as_str().to_string(),
        capability_hash: capability_hash(&capabilities),
        capability_stale: capabilities.stale,
        suite_hash: suite_hash(&cases),
        seed: options.seed,
        tool_count: capabilities.tools.len() as u64,
        resource_count: capabilities.resources.len() as u64,
        template_count: capabilities.resource_templates.len() as u64,
        prompt_count: capabilities.prompts.len() as u64,
        started_at,
        finished_at: None,
        options: options.clone(),
    };
    let run_id = Uuid::new_v4().to_string();
    let run = TestRun {
        id: run_id,
        suite_id,
        suite_name,
        server_id: server_id.to_string(),
        server_name: server.name,
        status: TestRunStatus::Running,
        stop_reason: None,
        options,
        manifest,
        summary: RunSummary {
            total: cases.len() as u64,
            coverage: coverage::compute(&cases, &capabilities),
            ..Default::default()
        },
        baseline_run_id: None,
        started_at,
        finished_at: None,
    };
    Ok(PreparedRun {
        run,
        capabilities,
        server_tags: server.tags,
    })
}

/// 启动后台运行任务（调用方已写入 test_runs 并设置 AppState.test_run）。
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    app: AppHandle,
    db: Database,
    manager: Arc<crate::mcp::ConnectionManager>,
    data_dir: PathBuf,
    prepared: PreparedRun,
    cases: Vec<TestCase>,
    control: RunControl,
    default_timeout_secs: u64,
) {
    tauri::async_runtime::spawn(async move {
        run_task(
            app,
            db,
            manager,
            data_dir,
            prepared,
            cases,
            control,
            default_timeout_secs,
        )
        .await;
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_task(
    app: AppHandle,
    db: Database,
    manager: Arc<crate::mcp::ConnectionManager>,
    data_dir: PathBuf,
    prepared: PreparedRun,
    cases: Vec<TestCase>,
    control: RunControl,
    default_timeout_secs: u64,
) {
    let run = prepared.run;
    let run_id = run.id.clone();
    let total = cases.len() as u64;
    let mut summary = run.summary.clone();
    let mut completed = 0u64;
    let mut canceled = false;
    let mut stop_reason: Option<String> = None;
    let started = Instant::now();
    let prod = prepared.server_tags.iter().any(|tag| {
        matches!(
            tag.to_ascii_lowercase().as_str(),
            "prod" | "production" | "生产"
        )
    });

    let _ = app.emit(
        "test://run-started",
        json!({ "run_id": run_id, "total": total }),
    );

    let Some(handle) = manager.handle(&run.server_id).await else {
        summary.error = total;
        finish(
            &app,
            &db,
            &run_id,
            TestRunStatus::Failed,
            Some("server_unavailable"),
            &summary,
            started.elapsed().as_millis() as u64,
        )
        .await;
        clear_control(&app, &run_id).await;
        return;
    };

    for (index, case) in cases.iter().enumerate() {
        if control.is_canceled() {
            canceled = true;
            stop_reason = Some("canceled".to_string());
            break;
        }
        let mut row = RunCaseRowBuilder::new(&run_id, case);
        if !case.enabled {
            row.skip("disabled");
            commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
            continue;
        }
        let effective_risk = effective_risk(case, &prepared.capabilities);
        let skip_reason = risk_skip_reason(&effective_risk, &run.options, prod);
        if let Some(reason) = skip_reason {
            row.skip(reason);
            commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
            continue;
        }
        if case.method != TestMethod::ResourcesRead && !target_exists(case, &prepared.capabilities) {
            row.skip("target_missing");
            commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
            continue;
        }

        let timeout_secs = case
            .timeout_secs
            .or(run.options.timeout_secs)
            .unwrap_or(default_timeout_secs)
            .max(1);
        let max_attempts = 1 + run.options.retry_count.min(3) + case.retry_count.min(3);
        let mut attempt_record_ids = Vec::new();
        let mut final_record: Option<CallRecord> = None;
        let mut final_eval: Option<assertions::Evaluation> = None;
        let mut evidence = Evidence {
            request: case.arguments.clone(),
            ..Default::default()
        };

        for attempt in 0..max_attempts {
            if control.is_canceled() {
                canceled = true;
                stop_reason = Some("canceled".to_string());
                break;
            }
            let call_id = Uuid::new_v4().to_string();
            control.set_current_call(Some(call_id.clone()));
            let wire_start = handle.wire().latest_seq();
            let notif_start = handle.hub().last_notification_seq();
            let stderr_start = handle.hub().last_stderr_seq();
            let record = call_case(&handle, case, call_id, timeout_secs).await;
            control.set_current_call(None);
            let persisted = persist_call(&app, &db, &data_dir, &record).await;
            attempt_record_ids.push(persisted.id.clone());
            let output_schema = prepared
                .capabilities
                .tools
                .iter()
                .find(|tool| tool.name == case.target)
                .and_then(|tool| tool.output_schema.as_ref());
            let evaluation =
                assertions::evaluate(&case.expect, &case.assertions, &persisted, output_schema);
            evidence = collect_evidence(
                &handle,
                wire_start,
                notif_start,
                stderr_start,
                &case.arguments,
                &persisted,
            );
            final_record = Some(persisted);
            final_eval = Some(evaluation.clone());
            let retryable = matches!(
                evaluation.failure_kind,
                Some(FailureKind::Timeout | FailureKind::TransportError)
            );
            if retryable && attempt + 1 < max_attempts && !control.is_canceled() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
            break;
        }

        let Some(record) = final_record else {
            // 取消发生在调用前
            row.skip("after_cancel");
            commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
            canceled = true;
            stop_reason = Some("canceled".to_string());
            break;
        };
        let evaluation = final_eval.unwrap_or(assertions::Evaluation {
            status: TestRunCaseStatus::Error,
            failure_kind: Some(FailureKind::CaseInvalid),
            assertions: Vec::new(),
            message: Some("用例没有产生结果".to_string()),
        });
        row.finish(
            record.status == CallStatus::Canceled || control.is_canceled(),
            evaluation,
            &attempt_record_ids,
            Some(record.id.clone()),
            &evidence,
            record.duration_ms,
        );
        commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
        if control.is_canceled() {
            canceled = true;
            stop_reason = Some("canceled".to_string());
            break;
        }
        if run.options.stop_on_failure
            && matches!(
                summary.failed + summary.error,
                value if value > 0
            )
        {
            stop_reason = Some("stop_on_failure".to_string());
            break;
        }
        if run.options.post_case_health
            && (index + 1) % 20 == 0
            && let Ok(result) = handle.probe().await
            && !result.ok
        {
            stop_reason = Some("server_unhealthy".to_string());
            break;
        }
    }

    // 剩余用例标记 skipped
    let executed = completed as usize;
    if executed < cases.len() {
        let reason = if canceled {
            "after_cancel"
        } else if stop_reason.as_deref() == Some("stop_on_failure") {
            "after_failure"
        } else {
            "skipped"
        };
        for case in cases.iter().skip(executed) {
            let mut row = RunCaseRowBuilder::new(&run_id, case);
            row.skip(reason);
            commit_case(&app, &db, &run_id, row, &mut summary, &mut completed, total).await;
        }
    }

    let status = if canceled {
        TestRunStatus::Canceled
    } else if summary.failed > 0 || summary.error > 0 {
        TestRunStatus::Failed
    } else {
        TestRunStatus::Completed
    };
    if canceled && stop_reason.is_none() {
        stop_reason = Some("canceled".to_string());
    }
    finish(
        &app,
        &db,
        &run_id,
        status,
        stop_reason.as_deref(),
        &summary,
        started.elapsed().as_millis() as u64,
    )
    .await;
    clear_control(&app, &run_id).await;
}

async fn call_case(
    handle: &ConnectionHandle,
    case: &TestCase,
    call_id: String,
    timeout_secs: u64,
) -> CallRecord {
    let options = CallOptions {
        call_id: Some(call_id),
        timeout_secs: Some(timeout_secs),
        origin: CallOrigin::Test,
    };
    match case.method {
        TestMethod::ToolsCall => {
            handle
                .call_tool_with_options(&case.target, case.arguments.clone(), options)
                .await
        }
        TestMethod::ResourcesRead => {
            handle
                .read_resource_with_options(&case.target, options)
                .await
        }
        TestMethod::PromptsGet => {
            handle
                .get_prompt_with_options(&case.target, case.arguments.clone(), options)
                .await
        }
    }
}

async fn persist_call(
    app: &AppHandle,
    db: &Database,
    data_dir: &Path,
    record: &CallRecord,
) -> CallRecord {
    let mut updated = record.clone();
    let results_dir = data_dir.join("results");
    let record_for_db = record.clone();
    match db
        .with_write(move |conn| call_repo::insert(conn, &results_dir, &record_for_db))
        .await
    {
        Ok(outcome) => {
            updated.truncated = outcome.truncated;
            updated.size_bytes = outcome.size_bytes;
        }
        Err(error) => {
            tracing::warn!(record_id = %record.id, error = %error, "测试调用记录落库失败");
            return updated;
        }
    }
    let summary = crate::mcp::models::CallRecordSummary {
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

fn evidence_error(result: Option<&Value>, error: &Option<CallError>) -> Option<CallError> {
    if result.is_none() {
        error.clone()
    } else {
        None
    }
}

fn collect_evidence(
    handle: &ConnectionHandle,
    wire_start: u64,
    notif_start: u64,
    stderr_start: u64,
    request: &Value,
    record: &CallRecord,
) -> Evidence {
    let mut evidence = Evidence {
        request: request.clone(),
        response_preview: record.result.clone(),
        response_error: evidence_error(record.result.as_ref(), &record.error),
        truncated: record.truncated,
        wire: handle.wire().since(wire_start),
        notifications: handle.hub().notification_buffer(notif_start),
        stderr: handle.hub().stderr_buffer(stderr_start),
        omitted: false,
    };
    // 单条证据上限 16KB；超出时只保留请求/响应摘要。
    if serde_json::to_string(&evidence)
        .map(|text| text.len())
        .unwrap_or(0)
        > 16 * 1024
    {
        evidence.wire.clear();
        evidence.notifications.clear();
        evidence.stderr.clear();
        evidence.omitted = true;
    }
    evidence
}

fn effective_risk(case: &TestCase, capabilities: &CapabilitySnapshot) -> TestRisk {
    let current = if case.method == TestMethod::ToolsCall {
        capabilities
            .tools
            .iter()
            .find(|tool| tool.name == case.target)
            .map(|tool| generator::classify_risk(tool).0)
            .unwrap_or(TestRisk::Unknown)
    } else {
        TestRisk::ReadOnly
    };
    let mut effective = case.risk.clone().max(current);
    if capabilities.stale && matches!(effective, TestRisk::Write | TestRisk::Unknown) {
        effective = TestRisk::Unknown;
    }
    effective
}

fn risk_skip_reason(risk: &TestRisk, options: &RunOptions, prod: bool) -> Option<&'static str> {
    match risk {
        TestRisk::ReadOnly => None,
        TestRisk::Write if options.allow_write => None,
        TestRisk::Write => Some("write_not_allowed"),
        TestRisk::Unknown if options.allow_unknown => None,
        TestRisk::Unknown => Some("unknown_not_allowed"),
        TestRisk::Destructive if options.allow_destructive && !prod => None,
        TestRisk::Destructive => Some("destructive_not_allowed"),
    }
}

fn target_exists(case: &TestCase, capabilities: &CapabilitySnapshot) -> bool {
    match case.method {
        TestMethod::ToolsCall => capabilities.tools.iter().any(|tool| tool.name == case.target),
        TestMethod::PromptsGet => capabilities
            .prompts
            .iter()
            .any(|prompt| prompt.name == case.target),
        TestMethod::ResourcesRead => true,
    }
}

async fn commit_case(
    app: &AppHandle,
    db: &Database,
    run_id: &str,
    row: RunCaseRowBuilder,
    summary: &mut RunSummary,
    completed: &mut u64,
    total: u64,
) {
    let row = row.build();
    let row_for_db = row.clone();
    match row.status {
        TestRunCaseStatus::Passed => summary.passed += 1,
        TestRunCaseStatus::Failed => summary.failed += 1,
        TestRunCaseStatus::Warning => summary.warning += 1,
        TestRunCaseStatus::Skipped => summary.skipped += 1,
        TestRunCaseStatus::Error => summary.error += 1,
        TestRunCaseStatus::Canceled => summary.canceled += 1,
    }
    summary.total = total;
    let summary_for_db = summary.clone();
    let run_id_for_db = run_id.to_string();
    if let Err(error) = db
        .with_write(move |conn| {
            let tx = conn.transaction()?;
            test_repo::insert_run_case(&tx, &row_for_db)?;
            test_repo::update_run_summary(&tx, &run_id_for_db, &summary_for_db)?;
            tx.commit()?;
            Ok(())
        })
        .await
    {
        tracing::warn!(run_id, error = %error, "测试运行结果落库失败");
    }
    *completed += 1;
    let _ = app.emit(
        "test://run-case-finished",
        json!({
            "run_id": run_id,
            "run_case_id": row.id,
            "case_name": row.case_name,
            "status": row.status,
            "failure_kind": row.failure_kind,
            "duration_ms": row.duration_ms,
        }),
    );
    let _ = app.emit(
        "test://run-progress",
        json!({
            "run_id": run_id,
            "completed": *completed,
            "total": total,
            "passed": summary.passed,
            "failed": summary.failed,
            "warning": summary.warning,
            "skipped": summary.skipped,
            "error": summary.error,
            "current": row.case_name,
        }),
    );
}

#[allow(clippy::too_many_arguments)]
async fn finish(
    app: &AppHandle,
    db: &Database,
    run_id: &str,
    status: TestRunStatus,
    stop_reason: Option<&str>,
    summary: &RunSummary,
    duration_ms: u64,
) {
    let mut summary = summary.clone();
    summary.duration_ms = duration_ms;
    summary.pass_rate = if summary.total > 0 {
        summary.passed as f64 / summary.total as f64
    } else {
        0.0
    };
    let denominator = summary.passed + summary.failed + summary.warning + summary.error;
    summary.effective_pass_rate = if denominator > 0 {
        summary.passed as f64 / denominator as f64
    } else {
        0.0
    };
    let run_id_for_db = run_id.to_string();
    let summary_for_db = summary.clone();
    let reason = stop_reason.map(ToString::to_string);
    let status_for_db = status.clone();
    if let Err(error) = db
        .with_write(move |conn| {
            test_repo::finish_run(
                conn,
                &run_id_for_db,
                status_for_db,
                reason.as_deref(),
                &summary_for_db,
            )
        })
        .await
    {
        tracing::warn!(run_id, error = %error, "测试运行收尾失败");
    }
    let _ = app.emit(
        "test://run-finished",
        json!({
            "run_id": run_id,
            "status": status,
            "summary": summary,
        }),
    );
}

async fn clear_control(app: &AppHandle, run_id: &str) {
    let state = app.state::<AppState>();
    let mut guard = state.test_run.lock().await;
    if guard
        .as_ref()
        .map(|control| control.run_id == run_id)
        .unwrap_or(false)
    {
        *guard = None;
    }
}

/// 运行用例行构造器：统一累计 summary。
struct RunCaseRowBuilder {
    id: String,
    run_id: String,
    case_id: Option<String>,
    case_name: String,
    method: TestMethod,
    target: String,
    order_index: i64,
    case_snapshot: Value,
    status: TestRunCaseStatus,
    failure_kind: Option<FailureKind>,
    skip_reason: Option<String>,
    duration_ms: u64,
    call_record_id: Option<String>,
    attempt_record_ids: Vec<String>,
    attempts: u32,
    assertions: Vec<AssertionResult>,
    evidence: Evidence,
    message: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

impl RunCaseRowBuilder {
    fn new(run_id: &str, case: &TestCase) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            case_id: Some(case.id.clone()),
            case_name: case.name.clone(),
            method: case.method.clone(),
            target: case.target.clone(),
            order_index: case.order_index,
            case_snapshot: serde_json::to_value(case).unwrap_or(Value::Null),
            status: TestRunCaseStatus::Skipped,
            failure_kind: None,
            skip_reason: None,
            duration_ms: 0,
            call_record_id: None,
            attempt_record_ids: Vec::new(),
            attempts: 0,
            assertions: Vec::new(),
            evidence: Evidence::default(),
            message: None,
            created_at: Utc::now(),
        }
    }

    fn skip(&mut self, reason: &str) {
        self.status = TestRunCaseStatus::Skipped;
        self.skip_reason = Some(reason.to_string());
        self.message = Some(format!("跳过：{reason}"));
    }

    fn finish(
        &mut self,
        canceled: bool,
        evaluation: assertions::Evaluation,
        attempt_record_ids: &[String],
        call_record_id: Option<String>,
        evidence: &Evidence,
        duration_ms: u64,
    ) {
        self.status = if canceled {
            TestRunCaseStatus::Canceled
        } else {
            evaluation.status.clone()
        };
        self.failure_kind = evaluation.failure_kind;
        self.duration_ms = duration_ms;
        self.call_record_id = call_record_id;
        self.attempt_record_ids = attempt_record_ids.to_vec();
        self.attempts = attempt_record_ids.len() as u32;
        self.assertions = evaluation.assertions;
        self.evidence = evidence.clone();
        self.message = evaluation.message;
    }

    fn build(self) -> test_repo::RunCaseRow {
        test_repo::RunCaseRow {
            id: self.id,
            run_id: self.run_id,
            case_id: self.case_id,
            case_name: self.case_name,
            method: self.method,
            target: self.target,
            order_index: self.order_index,
            case_snapshot: self.case_snapshot,
            status: self.status,
            failure_kind: self.failure_kind,
            skip_reason: self.skip_reason,
            duration_ms: self.duration_ms,
            call_record_id: self.call_record_id,
            attempt_record_ids: self.attempt_record_ids,
            attempts: self.attempts,
            assertions: self.assertions,
            evidence: self.evidence,
            message: self.message,
            created_at: self.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::evidence_error;
    use crate::mcp::error::CallError;
    use serde_json::json;

    #[test]
    fn response_error_only_when_result_missing() {
        let error = Some(CallError::Timeout { timeout_secs: 5 });
        assert!(evidence_error(None, &error).is_some());
        assert!(evidence_error(Some(&json!({"isError": true})), &error).is_none());
    }
}
