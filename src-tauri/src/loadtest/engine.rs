//! 压测引擎：阶段调度、VU worker、指标 tick、取消与收尾。
//!
//! P0 不依赖 DB/Tauri；事件通过可选的 `LoadEventSink` 回调输出。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::mcp::connection::{CallOptions, ConnectionHandle};
use crate::mcp::error::CallError;
use crate::mcp::input::RejectInputHandler;
use crate::mcp::models::{CallOrigin, CallStatus, ServerConfig};
use crate::mcp::oauth::OAuthService;
use crate::testing::assertions::result_is_error;
use crate::testing::models::TestMethod;

use super::metrics::{LoadMetrics, RollingStats, should_abort};
use super::models::{
    LoadCallStatus, LoadCaseSnapshot, LoadErrorSample, LoadProfile, LoadRunControl, LoadRunStatus,
    LoadSeriesPoint, LoadSummary, LoadTick, MAX_SERIES_POINTS,
};
use super::session::LoadSessionFactory;
use super::validate::pick_operation;

/// 事件出口：P0 不依赖 Tauri；P1 用 AppHandle 包一层。
pub type LoadEventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

pub struct LoadContext {
    pub run_id: String,
    pub server_id: String,
    pub server_name: String,
    pub profile: LoadProfile,
    pub operations: Vec<LoadCaseSnapshot>,
    pub config: ServerConfig,
    pub oauth: Option<Arc<OAuthService>>,
    /// 可选事件出口；测试传 None。
    pub event_sink: Option<LoadEventSink>,
}

pub struct LoadOutcome {
    pub status: LoadRunStatus,
    pub stop_reason: Option<String>,
    pub summary: LoadSummary,
    pub series: Vec<LoadSeriesPoint>,
    pub errors: Vec<LoadErrorSample>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// 运行压测。返回后调用方负责落库/推 finished；本函数只发 started/tick。
pub async fn run(context: LoadContext, control: Arc<LoadRunControl>) -> LoadOutcome {
    let started_at = Utc::now();
    let started = Instant::now();
    let metrics = Arc::new(LoadMetrics::new());
    let active_vus = Arc::new(AtomicU32::new(0));
    let connect_failures = Arc::new(AtomicU64::new(0));
    let sessions: Arc<Mutex<Vec<Arc<ConnectionHandle>>>> = Arc::new(Mutex::new(Vec::new()));
    let (target_tx, target_rx) = watch::channel(0u32);
    let (stop_tx, stop_rx) = watch::channel(false);
    let stage_index = Arc::new(AtomicU32::new(0));
    let stage_count = context.profile.stages.len() as u32;
    let max_vus = context.profile.max_vus();

    emit(
        context.event_sink.as_ref(),
        "loadtest://run-started",
        json!({
            "run_id": context.run_id,
            "duration_ms": context.profile.total_duration_secs() * 1000,
            "max_vus": max_vus,
            "session_mode": context.profile.session_mode,
        }),
    );

    let factory = Arc::new(LoadSessionFactory::new(
        context.config.clone(),
        &context.profile,
        context.oauth.clone(),
    ));

    // shared 模式由协调器创建唯一会话；worker 不负责断开。
    let shared = if context.profile.session_mode == super::models::LoadSessionMode::Shared {
        match connect_with_cancel(&factory, 0, &control).await {
            Ok(session) => {
                sessions
                    .lock()
                    .expect("压测会话表锁异常")
                    .push(Arc::clone(&session));
                Some(session)
            }
            Err(error) => {
                let summary = metrics.summary(started.elapsed().as_millis() as u64, max_vus);
                let _ = error;
                return LoadOutcome {
                    status: LoadRunStatus::Failed,
                    stop_reason: Some("connect_failed".to_string()),
                    summary,
                    series: Vec::new(),
                    errors: Vec::new(),
                    started_at,
                    finished_at: Utc::now(),
                };
            }
        }
    } else {
        None
    };

    let mut workers = JoinSet::new();
    for index in 0..max_vus {
        workers.spawn(run_worker(
            index,
            Arc::clone(&factory),
            shared.clone(),
            context.profile.clone(),
            context.operations.clone(),
            Arc::clone(&metrics),
            Arc::clone(&active_vus),
            target_rx.clone(),
            stop_rx.clone(),
            Arc::clone(&control),
            Arc::clone(&sessions),
            Arc::clone(&connect_failures),
            started,
        ));
    }

    let ticker = tokio::spawn(ticker_task(
        context.run_id.clone(),
        context.profile.clone(),
        Arc::clone(&metrics),
        Arc::clone(&active_vus),
        Arc::clone(&stage_index),
        stage_count,
        target_rx.clone(),
        stop_rx.clone(),
        Arc::clone(&control),
        Arc::clone(&connect_failures),
        context.event_sink.clone(),
        started,
    ));

    // 阶段调度（阶段内目标固定）。
    let mut canceled = false;
    for (index, stage) in context.profile.stages.iter().enumerate() {
        if control.is_canceled() {
            canceled = true;
            break;
        }
        stage_index.store(index as u32, Ordering::Relaxed);
        target_tx.send_replace(stage.target_vus);
        if wait_cancelable(
            Duration::from_secs(stage.duration_secs.max(1)),
            &control,
        )
        .await
        {
            canceled = true;
            break;
        }
    }
    if !canceled {
        stage_index.store(stage_count, Ordering::Relaxed);
        target_tx.send_replace(0);
    }
    stop_tx.send_replace(true);

    // 先给 worker 5s 自然退出；超时后并行断开全部会话，再等 1s，最后 abort。
    if tokio::time::timeout(Duration::from_secs(5), drain_workers(&mut workers))
        .await
        .is_err()
    {
        disconnect_all(&sessions).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), drain_workers(&mut workers)).await;
        workers.abort_all();
        while workers.join_next().await.is_some() {}
    }

    let mut series = match tokio::time::timeout(Duration::from_secs(2), ticker).await {
        Ok(Ok(series)) => series,
        Ok(Err(_)) => Vec::new(),
        Err(_) => Vec::new(),
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if series.len() < MAX_SERIES_POINTS {
        let (point, _) = metrics.snapshot(
            *target_rx.borrow(),
            active_vus.load(Ordering::Relaxed),
            stage_index.load(Ordering::Relaxed),
            elapsed_ms,
        );
        series.push(point);
    }

    let mut summary = metrics.summary(elapsed_ms, max_vus);
    summary.connect_failures = connect_failures.load(Ordering::Relaxed);
    drop(shared);
    disconnect_all(&sessions).await;

    let reason = control.reason();
    let status = match reason.as_deref() {
        Some("user") => LoadRunStatus::Canceled,
        Some(_) => LoadRunStatus::Aborted,
        None if summary.total == 0 && summary.connect_failures > 0 => LoadRunStatus::Failed,
        None => LoadRunStatus::Completed,
    };
    let stop_reason = reason.or_else(|| {
        (status == LoadRunStatus::Failed).then(|| "connect_failed".to_string())
    });

    let finished_at = Utc::now();

    LoadOutcome {
        status,
        stop_reason,
        summary,
        series,
        errors: metrics.all_errors(),
        started_at,
        finished_at,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    index: u32,
    factory: Arc<LoadSessionFactory>,
    shared: Option<Arc<ConnectionHandle>>,
    profile: LoadProfile,
    operations: Vec<LoadCaseSnapshot>,
    metrics: Arc<LoadMetrics>,
    active_vus: Arc<AtomicU32>,
    mut target_rx: watch::Receiver<u32>,
    mut stop_rx: watch::Receiver<bool>,
    control: Arc<LoadRunControl>,
    sessions: Arc<Mutex<Vec<Arc<ConnectionHandle>>>>,
    connect_failures: Arc<AtomicU64>,
    started: Instant,
) {
    let mut session = shared.clone();
    let is_shared = shared.is_some();
    let mut active = false;
    let mut rng = Rng::new(profile.seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut connect_error_count = 0u32;

    loop {
        if control.is_canceled() || *stop_rx.borrow() {
            break;
        }
        let target = *target_rx.borrow();
        if index >= target {
            if active {
                active_vus.fetch_sub(1, Ordering::Relaxed);
                active = false;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                _ = target_rx.changed() => {}
                _ = stop_rx.changed() => {}
                _ = control.canceled() => {}
            }
            continue;
        }
        if !active {
            active_vus.fetch_add(1, Ordering::Relaxed);
            active = true;
        }

        if session.is_none() {
            match connect_with_retry(&factory, index, &control).await {
                Ok(handle) => {
                    sessions
                        .lock()
                        .expect("压测会话表锁异常")
                        .push(Arc::clone(&handle));
                    session = Some(handle);
                    connect_error_count = 0;
                }
                Err(_) => {
                    connect_error_count += 1;
                    connect_failures.fetch_add(1, Ordering::Relaxed);
                    if connect_error_count >= 3 {
                        wait_for_target_or_stop(&mut target_rx, &mut stop_rx, &control).await;
                        connect_error_count = 0;
                    } else {
                        let _ = wait_cancelable(Duration::from_millis(500), &control).await;
                    }
                    continue;
                }
            }
        }

        let Some(handle) = session.as_ref() else {
            continue;
        };
        let operation_index = pick_operation(&operations, rng.next());
        let operation = &operations[operation_index];
        metrics.begin_call();
        let record = call_operation(handle, operation, &profile).await;
        metrics.end_call();

        let status = classify(&record, control.is_canceled());
        let message = record.error.as_ref().map(ToString::to_string);
        metrics.record(
            operation,
            status,
            record.duration_ms,
            record.size_bytes,
            message,
            started.elapsed().as_millis() as u64,
        );
        drop(record);

        let think_ms = if profile.think_time_max_ms > profile.think_time_min_ms {
            profile.think_time_min_ms
                + rng.range(profile.think_time_max_ms - profile.think_time_min_ms + 1)
        } else {
            profile.think_time_min_ms
        };
        if think_ms > 0 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(think_ms)) => {}
                _ = target_rx.changed() => {}
                _ = stop_rx.changed() => {}
                _ = control.canceled() => {}
            }
        }
    }

    if active {
        active_vus.fetch_sub(1, Ordering::Relaxed);
    }
    // shared 会话由协调器统一断开；per_vu worker 只释放自己的。
    if !is_shared
        && let Some(handle) = session.take()
    {
        handle.disconnect().await;
    }
}

async fn connect_with_retry(
    factory: &LoadSessionFactory,
    index: u32,
    control: &LoadRunControl,
) -> Result<Arc<ConnectionHandle>, String> {
    let mut last_error = String::new();
    for attempt in 0..3 {
        match connect_with_cancel(factory, index, control).await {
            Ok(handle) => return Ok(handle),
            Err(error) => {
                last_error = error;
                if attempt < 2 && wait_cancelable(Duration::from_millis(500), control).await {
                    return Err("canceled".to_string());
                }
            }
        }
    }
    Err(last_error)
}

async fn connect_with_cancel(
    factory: &LoadSessionFactory,
    index: u32,
    control: &LoadRunControl,
) -> Result<Arc<ConnectionHandle>, String> {
    tokio::select! {
        result = factory.connect(index) => result,
        _ = control.canceled() => Err("canceled".to_string()),
    }
}

async fn wait_for_target_or_stop(
    target_rx: &mut watch::Receiver<u32>,
    stop_rx: &mut watch::Receiver<bool>,
    control: &LoadRunControl,
) {
    tokio::select! {
        _ = target_rx.changed() => {}
        _ = stop_rx.changed() => {}
        _ = control.canceled() => {}
    }
}

/// 返回 true 表示被取消/停止。
async fn wait_cancelable(duration: Duration, control: &LoadRunControl) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = control.canceled() => true,
    }
}

async fn drain_workers(workers: &mut JoinSet<()>) {
    while workers.join_next().await.is_some() {}
}

async fn disconnect_all(sessions: &Arc<Mutex<Vec<Arc<ConnectionHandle>>>>) {
    let handles = {
        let guard = sessions.lock().expect("压测会话表锁异常");
        guard.clone()
    };
    if handles.is_empty() {
        return;
    }
    let futures = handles.into_iter().map(|handle| async move {
        handle.disconnect().await;
    });
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        futures::future::join_all(futures),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn ticker_task(
    run_id: String,
    profile: LoadProfile,
    metrics: Arc<LoadMetrics>,
    active_vus: Arc<AtomicU32>,
    stage_index: Arc<AtomicU32>,
    stage_count: u32,
    target_rx: watch::Receiver<u32>,
    mut stop_rx: watch::Receiver<bool>,
    control: Arc<LoadRunControl>,
    connect_failures: Arc<AtomicU64>,
    event_sink: Option<LoadEventSink>,
    started: Instant,
) -> Vec<LoadSeriesPoint> {
    let mut series = Vec::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await; // 跳过立即触发的首个 tick
    let idle_limit_ms = profile.idle_abort_ms();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let at_ms = started.elapsed().as_millis() as u64;
                let target = *target_rx.borrow();
                let active = active_vus.load(Ordering::Relaxed);
                let index = stage_index.load(Ordering::Relaxed);
                let (point, rolling) = metrics.snapshot(target, active, index, at_ms);
                if series.len() < MAX_SERIES_POINTS {
                    series.push(point.clone());
                }
                let (started_calls, completed, eligible, ok, errors, inflight) = metrics.cumulative();
                let tick = LoadTick {
                    run_id: run_id.clone(),
                    elapsed_ms: at_ms,
                    stage_index: index,
                    stage_count,
                    target_vus: target,
                    active_vus: active,
                    started_calls,
                    completed_calls: completed,
                    eligible,
                    ok,
                    errors,
                    inflight,
                    rps: point.rps,
                    p50_ms: point.p50_ms,
                    p95_ms: point.p95_ms,
                    p99_ms: point.p99_ms,
                    error_rate: point.error_rate,
                    recent_errors: metrics.recent_errors(),
                };
                control.update_latest(tick.clone());
                emit(event_sink.as_ref(), "loadtest://tick", json!(tick));

                if at_ms > 15_000
                    && metrics.total_completed() == 0
                    && connect_failures.load(Ordering::Relaxed) > 0
                {
                    control.request_cancel("connect_failed");
                    stop_rx.borrow_and_update();
                    return series;
                }
                if let Some(reason) = should_abort(
                    rolling,
                    profile.abort_error_rate,
                    profile.abort_p99_ms,
                    metrics.consecutive_transport_errors(),
                    metrics.idle_ms(at_ms),
                    idle_limit_ms,
                    metrics.inflight(),
                ) {
                    control.request_cancel(reason);
                    stop_rx.borrow_and_update();
                    return series;
                }
            }
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    return series;
                }
            }
            _ = control.canceled() => return series,
        }
    }
}

async fn call_operation(
    handle: &ConnectionHandle,
    operation: &LoadCaseSnapshot,
    profile: &LoadProfile,
) -> crate::mcp::models::CallRecord {
    let options = CallOptions {
        call_id: Some(uuid::Uuid::new_v4().to_string()),
        timeout_secs: Some(profile.timeout_secs),
        origin: CallOrigin::Test,
        input_handler: Some(Arc::new(RejectInputHandler::new())),
    };
    match operation.method {
        TestMethod::ToolsCall => {
            handle
                .call_tool_with_options(&operation.target, operation.arguments.clone(), options)
                .await
        }
        TestMethod::ResourcesRead => {
            handle
                .read_resource_with_options(&operation.target, options)
                .await
        }
        TestMethod::PromptsGet => {
            handle
                .get_prompt_with_options(&operation.target, operation.arguments.clone(), options)
                .await
        }
    }
}

fn classify(record: &crate::mcp::models::CallRecord, canceled: bool) -> LoadCallStatus {
    if record.status == CallStatus::Canceled {
        return LoadCallStatus::Canceled;
    }
    let tool_error = result_is_error(record.result.as_ref());
    let status = match &record.error {
        None if !tool_error => LoadCallStatus::Ok,
        None => LoadCallStatus::ToolError,
        Some(CallError::RpcError { code: 0, .. }) if tool_error => LoadCallStatus::ToolError,
        Some(CallError::RpcError { .. }) => LoadCallStatus::RpcError,
        Some(CallError::Timeout { .. }) => LoadCallStatus::Timeout,
        Some(CallError::TransportClosed { .. }) => LoadCallStatus::TransportError,
        Some(CallError::OAuthRequired { .. } | CallError::Forbidden { .. } | CallError::AuthTransient { .. }) => {
            LoadCallStatus::AuthError
        }
        Some(
            CallError::InputRequired
            | CallError::InputTimeout { .. }
            | CallError::InputUnsupported { .. }
            | CallError::InputInvalid { .. }
            | CallError::InputRoundsExceeded,
        ) => LoadCallStatus::InputRequired,
        Some(_) => LoadCallStatus::OtherError,
    };
    if canceled && status == LoadCallStatus::TransportError {
        LoadCallStatus::Canceled
    } else {
        status
    }
}

fn emit(sink: Option<&LoadEventSink>, event: &str, payload: serde_json::Value) {
    if let Some(sink) = sink {
        sink(event, payload);
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, max: u64) -> u64 {
        if max == 0 { 0 } else { self.next() % max }
    }
}

/// 让测试可以直接验证滚动窗口而不跑满 30s。
pub fn rolling_for_test(windows: &[RollingStats]) -> RollingStats {
    let mut result = RollingStats::default();
    for window in windows {
        result.eligible += window.eligible;
        result.errors += window.errors;
        result.p99_ms = result.p99_ms.max(window.p99_ms);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::CallRecord;
    use crate::testing::models::TestRisk;
    use serde_json::json;

    fn record(error: Option<CallError>, result: Option<serde_json::Value>) -> CallRecord {
        CallRecord {
            id: "c".to_string(),
            server_id: "s".to_string(),
            server_name: "s".to_string(),
            method: crate::mcp::models::CallMethod::ToolsCall,
            target: "echo".to_string(),
            params: json!({}),
            result,
            error,
            status: CallStatus::Error,
            duration_ms: 1,
            protocol_version: "2026-07-28".to_string(),
            transport: crate::mcp::models::TransportKind::Stdio,
            truncated: false,
            size_bytes: 0,
            origin: CallOrigin::Test,
            frames: None,
            rounds: None,
            created_at: Utc::now(),
        }
    }

    fn operation() -> LoadCaseSnapshot {
        LoadCaseSnapshot {
            case_id: "c1".to_string(),
            name: "echo".to_string(),
            method: TestMethod::ToolsCall,
            target: "echo".to_string(),
            arguments: json!({ "text": "hi" }),
            risk: TestRisk::ReadOnly,
            weight: 1,
            timeout_secs: None,
        }
    }

    #[test]
    fn classifies_tool_and_auth_errors() {
        let mut tool = record(
            Some(CallError::RpcError {
                code: 0,
                message: "工具返回 isError=true".to_string(),
                data: Some(json!({ "isError": true })),
            }),
            Some(json!({ "isError": true })),
        );
        tool.status = CallStatus::Error;
        assert_eq!(classify(&tool, false), LoadCallStatus::ToolError);
        let auth = record(
            Some(CallError::OAuthRequired {
                detail: None,
                required_scope: None,
                resource_metadata_url: None,
            }),
            None,
        );
        assert_eq!(classify(&auth, false), LoadCallStatus::AuthError);
        let canceled = record(Some(CallError::TransportClosed { reason: "x".into() }), None);
        assert_eq!(classify(&canceled, true), LoadCallStatus::Canceled);
    }

    #[test]
    fn weighted_selection_is_deterministic() {
        let a = operation();
        let mut b = a.clone();
        b.case_id = "c2".to_string();
        b.weight = 3;
        let operations = vec![a, b];
        let mut rng = Rng::new(42);
        let picks: Vec<usize> = (0..10)
            .map(|_| pick_operation(&operations, rng.next()))
            .collect();
        assert!(picks.iter().filter(|&&index| index == 1).count() > 3);
    }

    #[test]
    fn rolling_helper_merges() {
        let result = rolling_for_test(&[
            RollingStats {
                eligible: 10,
                errors: 2,
                p99_ms: 100,
            },
            RollingStats {
                eligible: 5,
                errors: 1,
                p99_ms: 200,
            },
        ]);
        assert_eq!(result.eligible, 15);
        assert_eq!(result.errors, 3);
        assert_eq!(result.p99_ms, 200);
    }
}

