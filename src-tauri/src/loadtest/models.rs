//! v0.7.0 MCP 压测领域模型（与 `src/types/loadtest.ts` 手工同步）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::models::TransportKind;
use crate::testing::models::{TestMethod, TestRisk};

/// 保护性硬上限：不是性能承诺。
pub const MAX_HTTP_VUS: u32 = 100;
pub const MAX_SSE_VUS: u32 = 20;
pub const MAX_STDIO_VUS: u32 = 20;
pub const MAX_SHARED_VUS: u32 = 100;
pub const MAX_OAUTH_PER_VU_VUS: u32 = 5;
pub const MAX_STAGES: usize = 12;
pub const MAX_STAGE_SECS: u64 = 3600;
pub const MAX_TOTAL_SECS: u64 = 7200;
pub const MAX_OPERATIONS: usize = 50;
pub const MAX_WEIGHT: u32 = 1000;
pub const MAX_THINK_MS: u64 = 60_000;
pub const MAX_TIMEOUT_SECS: u64 = 300;
pub const MAX_SERIES_POINTS: usize = 7200;
pub const MAX_ERROR_SAMPLES: usize = 50;
pub const MAX_TICK_ERRORS: usize = 10;
pub const GRACEFUL_STOP_SECS: u64 = 5;
pub const ABORT_WINDOW_TICKS: usize = 30;
pub const ABORT_MIN_ELIGIBLE: u64 = 20;
pub const MAX_CONSECUTIVE_TRANSPORT_ERRORS: u64 = 10;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadSessionMode {
    #[default]
    PerVu,
    Shared,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LoadStage {
    pub duration_secs: u64,
    pub target_vus: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LoadOperation {
    pub case_id: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

fn default_stages() -> Vec<LoadStage> {
    vec![LoadStage {
        duration_secs: 60,
        target_vus: 10,
    }]
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_seed() -> u64 {
    20261112
}

fn default_abort_error_rate() -> Option<f64> {
    Some(0.2)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadProfile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub session_mode: LoadSessionMode,
    #[serde(default = "default_stages")]
    pub stages: Vec<LoadStage>,
    #[serde(default)]
    pub think_time_min_ms: u64,
    #[serde(default)]
    pub think_time_max_ms: u64,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub operations: Vec<LoadOperation>,
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default)]
    pub allow_unknown: bool,
    #[serde(default = "default_abort_error_rate")]
    pub abort_error_rate: Option<f64>,
    #[serde(default)]
    pub abort_p99_ms: Option<u64>,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

impl Default for LoadProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            session_mode: LoadSessionMode::PerVu,
            stages: default_stages(),
            think_time_min_ms: 0,
            think_time_max_ms: 0,
            timeout_secs: default_timeout_secs(),
            operations: Vec::new(),
            allow_write: false,
            allow_unknown: false,
            abort_error_rate: default_abort_error_rate(),
            abort_p99_ms: None,
            seed: default_seed(),
        }
    }
}

impl LoadProfile {
    pub fn total_duration_secs(&self) -> u64 {
        self.stages.iter().map(|stage| stage.duration_secs).sum()
    }

    pub fn max_vus(&self) -> u32 {
        self.stages
            .iter()
            .map(|stage| stage.target_vus)
            .max()
            .unwrap_or(0)
    }

    pub fn idle_abort_ms(&self) -> u64 {
        (self.think_time_max_ms.saturating_mul(2))
            .saturating_add(self.timeout_secs.saturating_mul(1000))
            .max(10_000)
    }
}

/// 一次压测操作的快照（运行时不依赖用例后续编辑）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadCaseSnapshot {
    pub case_id: String,
    pub name: String,
    pub method: TestMethod,
    pub target: String,
    pub arguments: Value,
    pub risk: TestRisk,
    pub weight: u32,
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadCallStatus {
    Ok,
    ToolError,
    RpcError,
    Timeout,
    TransportError,
    AuthError,
    Canceled,
    InputRequired,
    OtherError,
}

impl LoadCallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LoadCallStatus::Ok => "ok",
            LoadCallStatus::ToolError => "tool_error",
            LoadCallStatus::RpcError => "rpc_error",
            LoadCallStatus::Timeout => "timeout",
            LoadCallStatus::TransportError => "transport_error",
            LoadCallStatus::AuthError => "auth_error",
            LoadCallStatus::Canceled => "canceled",
            LoadCallStatus::InputRequired => "input_required",
            LoadCallStatus::OtherError => "other_error",
        }
    }

    pub fn is_error(self) -> bool {
        !matches!(self, LoadCallStatus::Ok | LoadCallStatus::Canceled)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LoadOperationSummary {
    pub case_id: String,
    pub name: String,
    pub method: Option<TestMethod>,
    pub target: String,
    pub weight: u32,
    pub total: u64,
    pub eligible: u64,
    pub ok: u64,
    pub errors: u64,
    pub canceled: u64,
    pub success_rate: f64,
    pub avg_ms: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LoadSummary {
    pub total: u64,
    pub eligible: u64,
    pub ok: u64,
    pub tool_error: u64,
    pub rpc_error: u64,
    pub timeout: u64,
    pub transport_error: u64,
    pub auth_error: u64,
    pub canceled: u64,
    pub input_required: u64,
    pub other_error: u64,
    pub success_rate: f64,
    pub error_rate: f64,
    pub duration_ms: u64,
    pub rps: f64,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
    pub started_calls: u64,
    pub completed_calls: u64,
    pub inflight_at_end: u64,
    pub connect_failures: u64,
    pub result_bytes: u64,
    pub max_vus: u32,
    pub operations: Vec<LoadOperationSummary>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LoadSeriesPoint {
    /// 相对运行开始的毫秒数。
    pub at_ms: u64,
    pub stage_index: u32,
    pub target_vus: u32,
    pub active_vus: u32,
    pub completed_delta: u64,
    pub eligible_delta: u64,
    pub error_delta: u64,
    pub error_rate: f64,
    pub rps: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub inflight: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadErrorSample {
    pub at: DateTime<Utc>,
    pub case_name: String,
    pub method: TestMethod,
    pub target: String,
    pub status: LoadCallStatus,
    pub duration_ms: u64,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadRunStatus {
    Running,
    Completed,
    Failed,
    Canceled,
    Interrupted,
    Aborted,
}

impl LoadRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LoadRunStatus::Running => "running",
            LoadRunStatus::Completed => "completed",
            LoadRunStatus::Failed => "failed",
            LoadRunStatus::Canceled => "canceled",
            LoadRunStatus::Interrupted => "interrupted",
            LoadRunStatus::Aborted => "aborted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(LoadRunStatus::Running),
            "completed" => Some(LoadRunStatus::Completed),
            "failed" => Some(LoadRunStatus::Failed),
            "canceled" => Some(LoadRunStatus::Canceled),
            "interrupted" => Some(LoadRunStatus::Interrupted),
            "aborted" => Some(LoadRunStatus::Aborted),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LoadRiskSummary {
    pub read_only: u64,
    pub write: u64,
    pub unknown: u64,
    pub destructive: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadManifest {
    pub app_version: String,
    pub server_id: String,
    pub server_name: String,
    pub transport: String,
    pub protocol_version: String,
    pub lifecycle: String,
    pub capability_hash: String,
    pub capability_stale: bool,
    pub tool_count: u64,
    pub resource_count: u64,
    pub template_count: u64,
    pub prompt_count: u64,
    pub production: bool,
    pub risk_summary: LoadRiskSummary,
    pub started_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadRun {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub suite_id: String,
    pub suite_name: String,
    pub profile_name: String,
    pub status: LoadRunStatus,
    pub stop_reason: Option<String>,
    pub profile: LoadProfile,
    pub operations: Vec<LoadCaseSnapshot>,
    pub manifest: LoadManifest,
    pub summary: LoadSummary,
    pub series: Vec<LoadSeriesPoint>,
    pub errors: Vec<LoadErrorSample>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadRunSummary {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub suite_id: String,
    pub suite_name: String,
    pub profile_name: String,
    pub status: LoadRunStatus,
    pub stop_reason: Option<String>,
    pub total: u64,
    pub success_rate: f64,
    pub rps: f64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub duration_ms: u64,
    pub max_vus: u32,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadTick {
    pub run_id: String,
    pub elapsed_ms: u64,
    pub stage_index: u32,
    pub stage_count: u32,
    pub target_vus: u32,
    pub active_vus: u32,
    pub started_calls: u64,
    pub completed_calls: u64,
    pub eligible: u64,
    pub ok: u64,
    pub errors: u64,
    pub inflight: u64,
    pub rps: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub error_rate: f64,
    pub recent_errors: Vec<LoadErrorSample>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActiveLoadRun {
    pub run_id: String,
    pub server_id: String,
    pub suite_name: String,
    pub profile_name: String,
    pub started_at: DateTime<Utc>,
    pub latest: Option<LoadTick>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadPreviewOperation {
    pub case_id: String,
    pub name: String,
    pub method: TestMethod,
    pub target: String,
    pub weight: u32,
    pub effective_risk: TestRisk,
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadPreview {
    pub server_id: String,
    pub suite_id: String,
    pub production: bool,
    pub risk_summary: LoadRiskSummary,
    pub operations: Vec<LoadPreviewOperation>,
    pub warnings: Vec<String>,
    pub can_start: bool,
}

#[derive(Clone, Debug)]
pub struct PreparedLoad {
    pub run: LoadRun,
    pub operations: Vec<LoadCaseSnapshot>,
    pub manifest: LoadManifest,
    pub production: bool,
    pub risk_summary: LoadRiskSummary,
}

/// 压测运行控制（Arc 共享给 engine/cancel/get_active）。
pub struct LoadRunControl {
    pub run_id: String,
    pub server_id: String,
    pub cancel: AtomicBool,
    pub reason: Mutex<Option<String>>,
    pub latest: Mutex<Option<LoadTick>>,
    cancel_tx: tokio::sync::watch::Sender<bool>,
}

impl LoadRunControl {
    pub fn new(run_id: String, server_id: String) -> Arc<Self> {
        let (cancel_tx, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            run_id,
            server_id,
            cancel: AtomicBool::new(false),
            reason: Mutex::new(None),
            latest: Mutex::new(None),
            cancel_tx,
        })
    }

    pub fn request_cancel(&self, reason: impl Into<String>) {
        {
            let mut guard = self.reason.lock().expect("压测取消原因锁异常");
            if guard.is_none() {
                *guard = Some(reason.into());
            }
        }
        self.cancel.store(true, Ordering::SeqCst);
        self.cancel_tx.send_replace(true);
    }

    pub fn subscribe_cancel(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancel_tx.subscribe()
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn reason(&self) -> Option<String> {
        self.reason.lock().expect("压测取消原因锁异常").clone()
    }

    pub fn latest(&self) -> Option<LoadTick> {
        self.latest.lock().expect("压测最新指标锁异常").clone()
    }

    pub fn update_latest(&self, tick: LoadTick) {
        *self.latest.lock().expect("压测最新指标锁异常") = Some(tick);
    }

    pub async fn canceled(&self) {
        let mut rx = self.subscribe_cancel();
        if *rx.borrow() {
            return;
        }
        let _ = rx.changed().await;
    }
}

/// 传输上限（用于纯校验与前端提示）。
pub fn max_vus_for(transport: &TransportKind, shared: bool, oauth: bool) -> u32 {
    if shared {
        return MAX_SHARED_VUS;
    }
    match transport {
        TransportKind::Stdio => MAX_STDIO_VUS,
        TransportKind::Sse => MAX_SSE_VUS,
        TransportKind::StreamableHttp if oauth => MAX_OAUTH_PER_VU_VUS,
        TransportKind::StreamableHttp => MAX_HTTP_VUS,
    }
}
