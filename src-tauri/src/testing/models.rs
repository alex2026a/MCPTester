//! 测试领域模型（与 `src/types/testing.ts` 手工同步）。

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::error::CallError;
use crate::mcp::handler::{NotificationEntry, StderrLine};
use crate::mcp::models::CallMethod;
use crate::mcp::wire::WireEntry;

/// 单步用例支持的 MCP 方法（不包含 probe）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestMethod {
    ToolsCall,
    ResourcesRead,
    PromptsGet,
}

impl TestMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestMethod::ToolsCall => "tools_call",
            TestMethod::ResourcesRead => "resources_read",
            TestMethod::PromptsGet => "prompts_get",
        }
    }
}

impl From<TestMethod> for CallMethod {
    fn from(value: TestMethod) -> Self {
        match value {
            TestMethod::ToolsCall => CallMethod::ToolsCall,
            TestMethod::ResourcesRead => CallMethod::ResourcesRead,
            TestMethod::PromptsGet => CallMethod::PromptsGet,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestPriority {
    P0,
    #[default]
    P1,
    P2,
}

/// 风险等级；严格度用于运行时取 max。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestRisk {
    ReadOnly,
    Write,
    #[default]
    Unknown,
    Destructive,
}

impl TestRisk {
    pub fn strictness(&self) -> u8 {
        match self {
            TestRisk::ReadOnly => 0,
            TestRisk::Write => 1,
            TestRisk::Unknown => 2,
            TestRisk::Destructive => 3,
        }
    }

    pub fn max(self, other: Self) -> Self {
        if other.strictness() > self.strictness() {
            other
        } else {
            self
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestSource {
    #[default]
    Manual,
    Schema,
    History,
    Llm,
    Import,
}

/// 期望结果。`Rejected` 表示 tool_error 或 rpc_error 都算通过。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    #[default]
    Success,
    ToolError,
    RpcError,
    Rejected,
    Any,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    McpShape,
    OutputSchema,
    JsonPath,
    Content,
    Duration,
    Snapshot,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssertionSeverity {
    #[default]
    Error,
    Warning,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestRunStatus {
    Running,
    Completed,
    Failed,
    Canceled,
    Interrupted,
}

impl TestRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestRunStatus::Running => "running",
            TestRunStatus::Completed => "completed",
            TestRunStatus::Failed => "failed",
            TestRunStatus::Canceled => "canceled",
            TestRunStatus::Interrupted => "interrupted",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestRunCaseStatus {
    Passed,
    Failed,
    Warning,
    Skipped,
    Error,
    Canceled,
}

impl TestRunCaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestRunCaseStatus::Passed => "passed",
            TestRunCaseStatus::Failed => "failed",
            TestRunCaseStatus::Warning => "warning",
            TestRunCaseStatus::Skipped => "skipped",
            TestRunCaseStatus::Error => "error",
            TestRunCaseStatus::Canceled => "canceled",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    AssertionFailed,
    InvalidInputAccepted,
    ProtocolError,
    ToolErrorUnexpected,
    TransportError,
    Timeout,
    ServerUnhealthy,
    CaseInvalid,
    Unknown,
}

impl FailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureKind::AssertionFailed => "assertion_failed",
            FailureKind::InvalidInputAccepted => "invalid_input_accepted",
            FailureKind::ProtocolError => "protocol_error",
            FailureKind::ToolErrorUnexpected => "tool_error_unexpected",
            FailureKind::TransportError => "transport_error",
            FailureKind::Timeout => "timeout",
            FailureKind::ServerUnhealthy => "server_unhealthy",
            FailureKind::CaseInvalid => "case_invalid",
            FailureKind::Unknown => "unknown",
        }
    }
}

/// 用例期望。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestExpect {
    pub outcome: TestOutcome,
    #[serde(default)]
    pub rpc_code: Option<i64>,
    #[serde(default)]
    pub message_regex: Option<String>,
    /// 负向用例应拒绝但被接受时：true=warning，false=failed。
    #[serde(default = "default_true")]
    pub accept_as_warning: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TestExpect {
    fn default() -> Self {
        Self {
            outcome: TestOutcome::Success,
            rpc_code: None,
            message_regex: None,
            accept_as_warning: true,
        }
    }
}

/// 单条断言。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestAssertion {
    #[serde(default)]
    pub id: String,
    pub kind: AssertionKind,
    #[serde(default)]
    pub severity: AssertionSeverity,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub expected: Option<Value>,
    #[serde(default)]
    pub value_ms: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub ignore_paths: Vec<String>,
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub schema_ref: Option<String>,
}

/// 新建/编辑用例的草稿（不含 id / suite_id / 时间戳 / 排序）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestCaseDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub method: TestMethod,
    pub target: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    #[serde(default)]
    pub expect: TestExpect,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: TestPriority,
    #[serde(default)]
    pub risk: TestRisk,
    #[serde(default)]
    pub source: TestSource,
    #[serde(default)]
    pub generation_note: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub retry_count: u32,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

impl Default for TestCaseDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            method: TestMethod::ToolsCall,
            target: String::new(),
            arguments: empty_object(),
            expect: TestExpect::default(),
            assertions: Vec::new(),
            tags: Vec::new(),
            priority: TestPriority::P1,
            risk: TestRisk::Unknown,
            source: TestSource::Manual,
            generation_note: None,
            enabled: true,
            timeout_secs: None,
            retry_count: 0,
        }
    }
}

/// 持久化用例。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestCase {
    pub id: String,
    pub suite_id: String,
    pub name: String,
    pub description: String,
    pub method: TestMethod,
    pub target: String,
    pub arguments: Value,
    pub expect: TestExpect,
    pub assertions: Vec<TestAssertion>,
    pub tags: Vec<String>,
    pub priority: TestPriority,
    pub risk: TestRisk,
    pub source: TestSource,
    pub generation_note: Option<String>,
    pub enabled: bool,
    pub order_index: i64,
    pub timeout_secs: Option<u64>,
    pub retry_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestSuite {
    pub id: String,
    pub name: String,
    pub description: String,
    pub server_id: String,
    pub tags: Vec<String>,
    pub capability_hash: Option<String>,
    pub capability_snapshot: Option<Value>,
    pub baseline_run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestSuiteDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub server_id: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestSuiteUpdateDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestSuiteSummary {
    pub id: String,
    pub name: String,
    pub server_id: String,
    pub server_name: Option<String>,
    pub case_count: u64,
    pub enabled_count: u64,
    pub baseline_run_id: Option<String>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_run_status: Option<TestRunStatus>,
}

/// 运行选项。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunOptions {
    #[serde(default)]
    pub case_ids: Option<Vec<String>>,
    #[serde(default)]
    pub stop_on_failure: bool,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default)]
    pub allow_destructive: bool,
    #[serde(default)]
    pub allow_unknown: bool,
    #[serde(default)]
    pub post_case_health: bool,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

pub fn default_seed() -> u64 {
    20261021
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            case_ids: None,
            stop_on_failure: false,
            retry_count: 0,
            timeout_secs: None,
            allow_write: false,
            allow_destructive: false,
            allow_unknown: false,
            post_case_health: false,
            seed: default_seed(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CoverageCategory {
    pub name: String,
    pub covered: bool,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Coverage {
    pub tools_total: u64,
    pub tools_covered: u64,
    pub resources_total: u64,
    pub resources_covered: u64,
    pub prompts_total: u64,
    pub prompts_covered: u64,
    pub required_params_total: u64,
    pub required_params_covered: u64,
    pub constraints_total: u64,
    pub constraints_covered: u64,
    pub negative_categories: Vec<CoverageCategory>,
    pub assertion_cases: u64,
    pub selected_cases: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RunSummary {
    pub total: u64,
    pub passed: u64,
    pub failed: u64,
    pub warning: u64,
    pub skipped: u64,
    pub error: u64,
    pub canceled: u64,
    pub duration_ms: u64,
    pub pass_rate: f64,
    pub effective_pass_rate: f64,
    pub coverage: Coverage,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunManifest {
    pub app_version: String,
    pub server_id: String,
    pub server_name: String,
    pub protocol_version: String,
    pub lifecycle: String,
    pub transport: String,
    pub capability_hash: String,
    #[serde(default)]
    pub capability_stale: bool,
    pub suite_hash: String,
    pub seed: u64,
    pub tool_count: u64,
    pub resource_count: u64,
    pub template_count: u64,
    pub prompt_count: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub options: RunOptions,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestRun {
    pub id: String,
    pub suite_id: Option<String>,
    pub suite_name: String,
    pub server_id: String,
    pub server_name: String,
    pub status: TestRunStatus,
    pub stop_reason: Option<String>,
    pub options: RunOptions,
    pub manifest: RunManifest,
    pub summary: RunSummary,
    pub baseline_run_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestRunSummary {
    pub id: String,
    pub suite_id: Option<String>,
    pub suite_name: String,
    pub server_id: String,
    pub server_name: String,
    pub status: TestRunStatus,
    pub total: u64,
    pub passed: u64,
    pub failed: u64,
    pub warning: u64,
    pub skipped: u64,
    pub error: u64,
    pub canceled: u64,
    pub pass_rate: f64,
    pub effective_pass_rate: f64,
    pub duration_ms: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub baseline: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestRunDetail {
    pub run: TestRun,
    pub coverage: Coverage,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestRunCaseSummary {
    pub id: String,
    pub case_id: Option<String>,
    pub case_name: String,
    pub method: TestMethod,
    pub target: String,
    pub order_index: i64,
    pub status: TestRunCaseStatus,
    pub failure_kind: Option<FailureKind>,
    pub skip_reason: Option<String>,
    pub duration_ms: u64,
    pub attempts: u32,
    pub call_record_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssertionResult {
    pub assertion_id: String,
    pub kind: AssertionKind,
    pub severity: AssertionSeverity,
    pub passed: bool,
    pub expected: Option<Value>,
    pub actual: Option<Value>,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Evidence {
    #[serde(default)]
    pub request: Value,
    #[serde(default)]
    pub response_preview: Option<Value>,
    /// 没有响应体时的调用级错误（超时/传输关闭/JSON-RPC error 等）。
    /// 工具返回 isError=true 时结果仍在 response_preview 中，这里为 None。
    #[serde(default)]
    pub response_error: Option<CallError>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub wire: Vec<WireEntry>,
    #[serde(default)]
    pub notifications: Vec<NotificationEntry>,
    #[serde(default)]
    pub stderr: Vec<StderrLine>,
    #[serde(default)]
    pub omitted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TestRunCaseDetail {
    pub summary: TestRunCaseSummary,
    pub case_snapshot: Value,
    pub attempt_record_ids: Vec<String>,
    pub assertions: Vec<AssertionResult>,
    pub evidence: Evidence,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ComparisonItem {
    pub key: String,
    pub name: String,
    pub base_status: Option<TestRunCaseStatus>,
    pub target_status: Option<TestRunCaseStatus>,
    pub failure_kind: Option<FailureKind>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CoverageDelta {
    pub tools: f64,
    pub required_params: f64,
    pub negative_categories: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunComparison {
    pub base_run_id: String,
    pub target_run_id: String,
    pub new_failures: Vec<ComparisonItem>,
    pub fixed: Vec<ComparisonItem>,
    pub still_failing: Vec<ComparisonItem>,
    pub warning_changes: Vec<ComparisonItem>,
    pub error_cases: Vec<ComparisonItem>,
    pub new_cases: Vec<ComparisonItem>,
    pub removed_cases: Vec<ComparisonItem>,
    pub duration_delta_ms: i64,
    pub pass_rate_delta: f64,
    pub effective_pass_rate_delta: f64,
    pub coverage_delta: CoverageDelta,
    pub contract_changed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TestRunFilter {
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub suite_id: Option<String>,
    #[serde(default)]
    pub status: Option<TestRunStatus>,
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TestRunCaseFilter {
    #[serde(default)]
    pub status: Option<TestRunCaseStatus>,
    #[serde(default)]
    pub failure_kind: Option<FailureKind>,
    #[serde(default)]
    pub keyword: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContractSnapshot {
    pub server_id: String,
    pub capability_hash: String,
    pub fetched_at: DateTime<Utc>,
    pub stale: bool,
    pub tool_count: u64,
    pub resource_count: u64,
    pub template_count: u64,
    pub prompt_count: u64,
    pub snapshot: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContractDiff {
    pub changed: bool,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<ContractChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContractChange {
    pub kind: String,
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationScope {
    All,
    Tools,
    Resources,
    Prompts,
    Current,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GenerationProfile {
    Smoke,
    #[default]
    Standard,
    Thorough,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GenerationOptions {
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default = "default_scope")]
    pub scope: GenerationScope,
    #[serde(default)]
    pub profile: GenerationProfile,
    #[serde(default = "default_true")]
    pub include_boundary: bool,
    #[serde(default = "default_true")]
    pub include_negative: bool,
    #[serde(default = "default_true")]
    pub include_resources: bool,
    #[serde(default = "default_true")]
    pub include_prompts: bool,
    #[serde(default = "default_true")]
    pub use_history: bool,
    #[serde(default)]
    pub allow_destructive: bool,
    #[serde(default = "default_max_cases_per_tool")]
    pub max_cases_per_tool: u32,
    #[serde(default = "default_max_total")]
    pub max_total: u32,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

fn default_scope() -> GenerationScope {
    GenerationScope::All
}

fn default_max_cases_per_tool() -> u32 {
    12
}

fn default_max_total() -> u32 {
    300
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            target: None,
            scope: GenerationScope::All,
            profile: GenerationProfile::Standard,
            include_boundary: true,
            include_negative: true,
            include_resources: true,
            include_prompts: true,
            use_history: true,
            allow_destructive: false,
            max_cases_per_tool: 12,
            max_total: 300,
            seed: default_seed(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GenerationStats {
    pub total: u32,
    pub positive: u32,
    pub boundary: u32,
    pub negative: u32,
    pub resources: u32,
    pub prompts: u32,
    pub read_only: u32,
    pub write: u32,
    pub destructive: u32,
    pub unknown: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GenerationPreview {
    pub server_id: String,
    pub capability_hash: String,
    pub capability_snapshot: Value,
    pub cases: Vec<TestCaseDraft>,
    pub warnings: Vec<String>,
    pub stats: GenerationStats,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportTestIssue {
    pub index: u32,
    pub name: Option<String>,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportTestCasesReport {
    pub added: u32,
    pub skipped: u32,
    pub failed: Vec<ImportTestIssue>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActiveTestRun {
    pub run_id: String,
    pub server_id: String,
    pub total: u64,
    pub completed: u64,
    pub started_at: DateTime<Utc>,
}

/// 运行控制（不序列化）：取消标志 + 当前调用 id。
#[derive(Clone)]
pub struct RunControl {
    pub run_id: String,
    pub server_id: String,
    pub cancel: Arc<AtomicBool>,
    pub current_call_id: Arc<Mutex<Option<String>>>,
}

impl RunControl {
    pub fn new(run_id: impl Into<String>, server_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            server_id: server_id.into(),
            cancel: Arc::new(AtomicBool::new(false)),
            current_call_id: Arc::new(Mutex::new(None)),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_current_call(&self, call_id: Option<String>) {
        if let Ok(mut guard) = self.current_call_id.lock() {
            *guard = call_id;
        }
    }

    pub fn current_call(&self) -> Option<String> {
        self.current_call_id.lock().ok().and_then(|guard| guard.clone())
    }
}
