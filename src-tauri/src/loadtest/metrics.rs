//! MCP 级压测指标：全局/每秒/30s 滚动窗口。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use hdrhistogram::Histogram;

use super::models::{
    ABORT_WINDOW_TICKS, LoadCallStatus, LoadCaseSnapshot, LoadErrorSample, LoadOperationSummary,
    LoadSeriesPoint, LoadSummary, MAX_ERROR_SAMPLES, MAX_SERIES_POINTS, MAX_TICK_ERRORS,
};
use super::validate::clamp_error_message;

const MAX_HIST_MS: u64 = 300_000;

struct OpMetrics {
    case_id: String,
    name: String,
    method: crate::testing::models::TestMethod,
    target: String,
    weight: u32,
    total: u64,
    eligible: u64,
    ok: u64,
    errors: u64,
    canceled: u64,
    sum_ms: u64,
    hist: Histogram<u64>,
}

struct TickWindow {
    eligible: u64,
    errors: u64,
    hist: Histogram<u64>,
}

struct MetricsInner {
    started: u64,
    completed: u64,
    inflight: u64,
    status: [u64; 9],
    result_bytes: u64,
    hist: Histogram<u64>,
    tick_hist: Histogram<u64>,
    recent: VecDeque<TickWindow>,
    per_op: HashMap<String, OpMetrics>,
    errors: VecDeque<LoadErrorSample>,
    consecutive_transport_errors: u64,
    last_completed: u64,
    last_eligible: u64,
    last_errors: u64,
    last_at_ms: u64,
    last_success_ms: Option<u64>,
}

pub struct LoadMetrics {
    inner: Mutex<MetricsInner>,
}

impl Default for LoadMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadMetrics {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MetricsInner {
                started: 0,
                completed: 0,
                inflight: 0,
                status: [0; 9],
                result_bytes: 0,
                hist: new_hist(),
                tick_hist: new_hist(),
                recent: VecDeque::new(),
                per_op: HashMap::new(),
                errors: VecDeque::new(),
                consecutive_transport_errors: 0,
                last_completed: 0,
                last_eligible: 0,
                last_errors: 0,
                last_at_ms: 0,
                last_success_ms: None,
            }),
        }
    }

    pub fn begin_call(&self) {
        let mut inner = self.inner.lock().expect("压测指标锁异常");
        inner.started += 1;
        inner.inflight += 1;
    }

    pub fn end_call(&self) {
        let mut inner = self.inner.lock().expect("压测指标锁异常");
        inner.inflight = inner.inflight.saturating_sub(1);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        operation: &LoadCaseSnapshot,
        status: LoadCallStatus,
        duration_ms: u64,
        result_bytes: u64,
        message: Option<String>,
        at_ms: u64,
    ) {
        let duration_ms = duration_ms.min(MAX_HIST_MS);
        let mut inner = self.inner.lock().expect("压测指标锁异常");
        inner.completed += 1;
        inner.result_bytes = inner.result_bytes.saturating_add(result_bytes);
        let _ = inner.hist.record(duration_ms);
        let _ = inner.tick_hist.record(duration_ms);

        {
            let op = inner
                .per_op
                .entry(operation.case_id.clone())
                .or_insert_with(|| OpMetrics {
                    case_id: operation.case_id.clone(),
                    name: operation.name.clone(),
                    method: operation.method.clone(),
                    target: operation.target.clone(),
                    weight: operation.weight,
                    total: 0,
                    eligible: 0,
                    ok: 0,
                    errors: 0,
                    canceled: 0,
                    sum_ms: 0,
                    hist: new_hist(),
                });
            op.total += 1;
            op.sum_ms = op.sum_ms.saturating_add(duration_ms);
            let _ = op.hist.record(duration_ms);
        }

        let slot = status_index(status);
        inner.status[slot] += 1;
        match status {
            LoadCallStatus::Ok => {
                inner.last_success_ms = Some(at_ms);
                inner.consecutive_transport_errors = 0;
            }
            LoadCallStatus::Canceled => {
                inner.consecutive_transport_errors = 0;
            }
            _ => {
                if status == LoadCallStatus::TransportError {
                    inner.consecutive_transport_errors += 1;
                } else {
                    inner.consecutive_transport_errors = 0;
                }
                if inner.errors.len() >= MAX_ERROR_SAMPLES {
                    inner.errors.pop_front();
                }
                inner.errors.push_back(LoadErrorSample {
                    at: chrono::Utc::now(),
                    case_name: operation.name.clone(),
                    method: operation.method.clone(),
                    target: operation.target.clone(),
                    status,
                    duration_ms,
                    message: clamp_error_message(
                        message.as_deref().unwrap_or(status.as_str()),
                    ),
                });
            }
        }
        if let Some(op) = inner.per_op.get_mut(&operation.case_id) {
            match status {
                LoadCallStatus::Ok => {
                    op.ok += 1;
                    op.eligible += 1;
                }
                LoadCallStatus::Canceled => op.canceled += 1,
                _ => {
                    op.errors += 1;
                    op.eligible += 1;
                }
            }
        }
    }

    /// 生成 1s 时间序列点；返回 (point, 滚动窗口 eligible/errors/p99)。
    pub fn snapshot(
        &self,
        target_vus: u32,
        active_vus: u32,
        stage_index: u32,
        at_ms: u64,
    ) -> (LoadSeriesPoint, RollingStats) {
        let mut inner = self.inner.lock().expect("压测指标锁异常");
        let completed_delta = inner.completed.saturating_sub(inner.last_completed);
        let eligible_delta = (inner.completed.saturating_sub(inner.status[6]))
            .saturating_sub(inner.last_eligible);
        let error_delta = inner
            .completed
            .saturating_sub(inner.status[0])
            .saturating_sub(inner.status[6])
            .saturating_sub(inner.last_errors);
        let interval_ms = at_ms.saturating_sub(inner.last_at_ms).max(1);
        let interval_secs = interval_ms as f64 / 1000.0;
        let rps = if eligible_delta > 0 {
            eligible_delta as f64 / interval_secs
        } else {
            0.0
        };
        let error_rate = if eligible_delta > 0 {
            error_delta as f64 / eligible_delta as f64
        } else {
            0.0
        };

        let point = LoadSeriesPoint {
            at_ms,
            stage_index,
            target_vus,
            active_vus,
            completed_delta,
            eligible_delta,
            error_delta,
            error_rate,
            rps,
            p50_ms: quantile(&inner.tick_hist, 0.50),
            p95_ms: quantile(&inner.tick_hist, 0.95),
            p99_ms: quantile(&inner.tick_hist, 0.99),
            inflight: inner.inflight,
        };

        let tick_hist = std::mem::replace(&mut inner.tick_hist, new_hist());
        inner.recent.push_back(TickWindow {
            eligible: eligible_delta,
            errors: error_delta,
            hist: tick_hist,
        });
        while inner.recent.len() > ABORT_WINDOW_TICKS {
            inner.recent.pop_front();
        }

        inner.last_completed = inner.completed;
        inner.last_eligible = inner.completed.saturating_sub(inner.status[6]);
        inner.last_errors = inner
            .completed
            .saturating_sub(inner.status[0])
            .saturating_sub(inner.status[6]);
        inner.last_at_ms = at_ms;

        let rolling = merge_rolling(&inner.recent);
        (point, rolling)
    }

    pub fn summary(&self, duration_ms: u64, max_vus: u32) -> LoadSummary {
        let inner = self.inner.lock().expect("压测指标锁异常");
        let completed = inner.completed;
        let canceled = inner.status[6];
        let eligible = completed.saturating_sub(canceled);
        let errors = eligible.saturating_sub(inner.status[0]);
        let duration_secs = duration_ms.max(1) as f64 / 1000.0;

        let mut operations: Vec<LoadOperationSummary> = inner
            .per_op
            .values()
            .map(|op| {
                let op_eligible = op.eligible;
                LoadOperationSummary {
                    case_id: op.case_id.clone(),
                    name: op.name.clone(),
                    method: Some(op.method.clone()),
                    target: op.target.clone(),
                    weight: op.weight,
                    total: op.total,
                    eligible: op_eligible,
                    ok: op.ok,
                    errors: op.errors,
                    canceled: op.canceled,
                    success_rate: ratio(op.ok, op_eligible),
                    avg_ms: if op.total > 0 {
                        op.sum_ms as f64 / op.total as f64
                    } else {
                        0.0
                    },
                    p50_ms: quantile(&op.hist, 0.50),
                    p95_ms: quantile(&op.hist, 0.95),
                    p99_ms: quantile(&op.hist, 0.99),
                }
            })
            .collect();
        operations.sort_by(|a, b| a.name.cmp(&b.name));

        LoadSummary {
            total: completed,
            eligible,
            ok: inner.status[0],
            tool_error: inner.status[1],
            rpc_error: inner.status[2],
            timeout: inner.status[3],
            transport_error: inner.status[4],
            auth_error: inner.status[5],
            canceled,
            input_required: inner.status[7],
            other_error: inner.status[8],
            success_rate: ratio(inner.status[0], eligible),
            error_rate: ratio(errors, eligible),
            duration_ms,
            rps: if duration_ms > 0 {
                eligible as f64 / duration_secs
            } else {
                0.0
            },
            p50_ms: quantile(&inner.hist, 0.50),
            p90_ms: quantile(&inner.hist, 0.90),
            p95_ms: quantile(&inner.hist, 0.95),
            p99_ms: quantile(&inner.hist, 0.99),
            max_ms: inner.hist.max(),
            started_calls: inner.started,
            completed_calls: completed,
            inflight_at_end: inner.inflight,
            connect_failures: 0,
            result_bytes: inner.result_bytes,
            max_vus,
            operations,
        }
    }

    pub fn recent_errors(&self) -> Vec<LoadErrorSample> {
        let inner = self.inner.lock().expect("压测指标锁异常");
        inner
            .errors
            .iter()
            .rev()
            .take(MAX_TICK_ERRORS)
            .cloned()
            .collect()
    }

    pub fn all_errors(&self) -> Vec<LoadErrorSample> {
        let inner = self.inner.lock().expect("压测指标锁异常");
        inner.errors.iter().cloned().collect()
    }

    pub fn consecutive_transport_errors(&self) -> u64 {
        self.inner
            .lock()
            .expect("压测指标锁异常")
            .consecutive_transport_errors
    }

    pub fn inflight(&self) -> u64 {
        self.inner.lock().expect("压测指标锁异常").inflight
    }

    pub fn idle_ms(&self, at_ms: u64) -> u64 {
        let inner = self.inner.lock().expect("压测指标锁异常");
        at_ms.saturating_sub(inner.last_success_ms.unwrap_or(0))
    }

    pub fn total_completed(&self) -> u64 {
        self.inner.lock().expect("压测指标锁异常").completed
    }

    /// (started, completed, eligible, ok, errors, inflight)
    pub fn cumulative(&self) -> (u64, u64, u64, u64, u64, u64) {
        let inner = self.inner.lock().expect("压测指标锁异常");
        let completed = inner.completed;
        let canceled = inner.status[6];
        let eligible = completed.saturating_sub(canceled);
        let ok = inner.status[0];
        let errors = eligible.saturating_sub(ok);
        (inner.started, completed, eligible, ok, errors, inner.inflight)
    }

    pub fn series_capacity_reached(series_len: usize) -> bool {
        series_len >= MAX_SERIES_POINTS
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RollingStats {
    pub eligible: u64,
    pub errors: u64,
    pub p99_ms: u64,
}

fn new_hist() -> Histogram<u64> {
    Histogram::new(3).expect("HDR 直方图创建失败")
}

fn quantile(hist: &Histogram<u64>, q: f64) -> u64 {
    if hist.is_empty() {
        0
    } else {
        hist.value_at_quantile(q)
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn status_index(status: LoadCallStatus) -> usize {
    match status {
        LoadCallStatus::Ok => 0,
        LoadCallStatus::ToolError => 1,
        LoadCallStatus::RpcError => 2,
        LoadCallStatus::Timeout => 3,
        LoadCallStatus::TransportError => 4,
        LoadCallStatus::AuthError => 5,
        LoadCallStatus::Canceled => 6,
        LoadCallStatus::InputRequired => 7,
        LoadCallStatus::OtherError => 8,
    }
}

fn merge_rolling(recent: &VecDeque<TickWindow>) -> RollingStats {
    let mut eligible = 0u64;
    let mut errors = 0u64;
    let mut hist = new_hist();
    for window in recent {
        eligible += window.eligible;
        errors += window.errors;
        let _ = hist.add(&window.hist);
    }
    RollingStats {
        eligible,
        errors,
        p99_ms: quantile(&hist, 0.99),
    }
}

/// 判断是否需要按滚动窗口中止。
pub fn should_abort(
    rolling: RollingStats,
    error_rate: Option<f64>,
    p99_ms: Option<u64>,
    consecutive_transport_errors: u64,
    idle_ms: u64,
    idle_limit_ms: u64,
    inflight: u64,
) -> Option<&'static str> {
    if rolling.eligible >= super::models::ABORT_MIN_ELIGIBLE {
        if let Some(limit) = error_rate
            && ratio(rolling.errors, rolling.eligible) > limit
        {
            return Some("error_rate");
        }
        if let Some(limit) = p99_ms
            && rolling.p99_ms > limit
        {
            return Some("p99");
        }
    }
    if consecutive_transport_errors >= super::models::MAX_CONSECUTIVE_TRANSPORT_ERRORS {
        return Some("transport_error");
    }
    if inflight == 0 && idle_ms > idle_limit_ms {
        return Some("idle");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::models::{TestMethod, TestRisk};
    use serde_json::json;

    fn op() -> LoadCaseSnapshot {
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
    fn records_and_summarizes() {
        let metrics = LoadMetrics::new();
        for index in 0..10 {
            metrics.begin_call();
            metrics.record(
                &op(),
                if index == 0 {
                    LoadCallStatus::ToolError
                } else {
                    LoadCallStatus::Ok
                },
                10 + index,
                100,
                None,
                index * 100,
            );
            metrics.end_call();
        }
        let summary = metrics.summary(1000, 10);
        assert_eq!(summary.total, 10);
        assert_eq!(summary.ok, 9);
        assert_eq!(summary.tool_error, 1);
        assert_eq!(summary.eligible, 10);
        assert!((summary.success_rate - 0.9).abs() < 1e-9);
        assert_eq!(summary.operations.len(), 1);
        assert_eq!(summary.operations[0].eligible, 10);
        assert_eq!(metrics.all_errors().len(), 1);
    }

    #[test]
    fn snapshot_uses_tick_percentiles_and_rolling() {
        let metrics = LoadMetrics::new();
        for index in 0..25 {
            metrics.begin_call();
            metrics.record(
                &op(),
                if index == 0 {
                    LoadCallStatus::Timeout
                } else {
                    LoadCallStatus::Ok
                },
                100 + index,
                0,
                None,
                100,
            );
            metrics.end_call();
        }
        let (point, rolling) = metrics.snapshot(5, 5, 0, 1000);
        assert_eq!(point.eligible_delta, 25);
        assert_eq!(point.error_delta, 1);
        assert!(point.p99_ms >= 100);
        assert_eq!(rolling.eligible, 25);
        assert_eq!(rolling.errors, 1);
    }

    #[test]
    fn canceled_is_not_in_eligible() {
        let metrics = LoadMetrics::new();
        metrics.record(&op(), LoadCallStatus::Canceled, 5, 0, None, 1);
        let summary = metrics.summary(1000, 1);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.eligible, 0);
        assert_eq!(summary.success_rate, 0.0);
    }

    #[test]
    fn abort_thresholds() {
        let rolling = RollingStats {
            eligible: 100,
            errors: 50,
            p99_ms: 900,
        };
        assert_eq!(
            should_abort(rolling, Some(0.2), None, 0, 0, 10_000, 1),
            Some("error_rate")
        );
        assert_eq!(
            should_abort(rolling, None, Some(500), 0, 0, 10_000, 1),
            Some("p99")
        );
        assert_eq!(should_abort(rolling, None, None, 12, 0, 10_000, 1), Some("transport_error"));
        assert_eq!(should_abort(rolling, None, None, 0, 20_000, 10_000, 0), Some("idle"));
        assert_eq!(should_abort(rolling, None, None, 0, 20_000, 10_000, 1), None);
    }

    #[test]
    fn error_samples_are_capped_and_sanitized() {
        let metrics = LoadMetrics::new();
        for index in 0..60 {
            metrics.record(
                &op(),
                LoadCallStatus::RpcError,
                10,
                0,
                Some(format!("Bearer secret {index}")),
                index,
            );
        }
        let errors = metrics.all_errors();
        assert_eq!(errors.len(), MAX_ERROR_SAMPLES);
        assert!(errors.iter().all(|item| !item.message.contains("secret")));
    }
}
