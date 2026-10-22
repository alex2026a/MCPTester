/** v0.2.2 测试领域类型（与 src-tauri/src/testing/models.rs 手工同步）。 */

import type { WireEntry } from "../api/wire";
import type { CallError } from "./models";

export type TestMethod = "tools_call" | "resources_read" | "prompts_get";
export type TestPriority = "p0" | "p1" | "p2";
export type TestRisk = "read_only" | "write" | "unknown" | "destructive";
export type TestSource = "manual" | "schema" | "history" | "llm" | "import";
export type TestOutcome = "success" | "tool_error" | "rpc_error" | "rejected" | "any";
export type AssertionKind =
  | "mcp_shape"
  | "output_schema"
  | "json_path"
  | "content"
  | "duration"
  | "snapshot";
export type AssertionSeverity = "error" | "warning";
export type TestRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "canceled"
  | "interrupted";
export type TestRunCaseStatus =
  | "passed"
  | "failed"
  | "warning"
  | "skipped"
  | "error"
  | "canceled";
export type FailureKind =
  | "assertion_failed"
  | "invalid_input_accepted"
  | "protocol_error"
  | "tool_error_unexpected"
  | "transport_error"
  | "timeout"
  | "server_unhealthy"
  | "case_invalid"
  | "unknown";

export interface TestExpect {
  outcome: TestOutcome;
  rpc_code?: number | null;
  message_regex?: string | null;
  accept_as_warning: boolean;
}

export interface TestAssertion {
  id: string;
  kind: AssertionKind;
  severity: AssertionSeverity;
  enabled: boolean;
  target?: string | null;
  path?: string | null;
  op?: string | null;
  expected?: unknown;
  value_ms?: number | null;
  message?: string | null;
  ignore_paths: string[];
  schema?: unknown;
  schema_ref?: string | null;
}

export interface TestCaseDraft {
  name: string;
  description: string;
  method: TestMethod;
  target: string;
  arguments: unknown;
  expect: TestExpect;
  assertions: TestAssertion[];
  tags: string[];
  priority: TestPriority;
  risk: TestRisk;
  source: TestSource;
  generation_note?: string | null;
  enabled: boolean;
  timeout_secs?: number | null;
  retry_count: number;
}

export interface TestCase extends TestCaseDraft {
  id: string;
  suite_id: string;
  order_index: number;
  created_at: string;
  updated_at: string;
}

export interface TestSuite {
  id: string;
  name: string;
  description: string;
  server_id: string;
  tags: string[];
  capability_hash?: string | null;
  capability_snapshot?: unknown | null;
  baseline_run_id?: string | null;
  created_at: string;
  updated_at: string;
}

export interface TestSuiteDraft {
  name: string;
  description: string;
  server_id: string;
  tags: string[];
}

export interface TestSuiteUpdateDraft {
  name: string;
  description: string;
  tags: string[];
}

export interface TestSuiteSummary {
  id: string;
  name: string;
  server_id: string;
  server_name?: string | null;
  case_count: number;
  enabled_count: number;
  baseline_run_id?: string | null;
  last_run_at?: string | null;
  last_run_status?: TestRunStatus | null;
}

export interface RunOptions {
  case_ids?: string[] | null;
  stop_on_failure: boolean;
  retry_count: number;
  timeout_secs?: number | null;
  allow_write: boolean;
  allow_destructive: boolean;
  allow_unknown: boolean;
  post_case_health: boolean;
  seed: number;
}

export interface CoverageCategory {
  name: string;
  covered: boolean;
  count: number;
}

export interface Coverage {
  tools_total: number;
  tools_covered: number;
  resources_total: number;
  resources_covered: number;
  prompts_total: number;
  prompts_covered: number;
  required_params_total: number;
  required_params_covered: number;
  constraints_total: number;
  constraints_covered: number;
  negative_categories: CoverageCategory[];
  assertion_cases: number;
  selected_cases: number;
}

export interface RunSummary {
  total: number;
  passed: number;
  failed: number;
  warning: number;
  skipped: number;
  error: number;
  canceled: number;
  duration_ms: number;
  pass_rate: number;
  effective_pass_rate: number;
  coverage: Coverage;
}

export interface RunManifest {
  app_version: string;
  server_id: string;
  server_name: string;
  protocol_version: string;
  lifecycle: string;
  transport: string;
  capability_hash: string;
  capability_stale: boolean;
  suite_hash: string;
  seed: number;
  tool_count: number;
  resource_count: number;
  template_count: number;
  prompt_count: number;
  started_at: string;
  finished_at?: string | null;
  options: RunOptions;
}

export interface TestRun {
  id: string;
  suite_id?: string | null;
  suite_name: string;
  server_id: string;
  server_name: string;
  status: TestRunStatus;
  stop_reason?: string | null;
  options: RunOptions;
  manifest: RunManifest;
  summary: RunSummary;
  baseline_run_id?: string | null;
  started_at: string;
  finished_at?: string | null;
}

export interface TestRunSummary {
  id: string;
  suite_id?: string | null;
  suite_name: string;
  server_id: string;
  server_name: string;
  status: TestRunStatus;
  total: number;
  passed: number;
  failed: number;
  warning: number;
  skipped: number;
  error: number;
  canceled: number;
  pass_rate: number;
  effective_pass_rate: number;
  duration_ms: number;
  started_at: string;
  finished_at?: string | null;
  baseline: boolean;
}

export interface TestRunDetail {
  run: TestRun;
  coverage: Coverage;
}

export interface TestRunCaseSummary {
  id: string;
  case_id?: string | null;
  case_name: string;
  method: TestMethod;
  target: string;
  order_index: number;
  status: TestRunCaseStatus;
  failure_kind?: FailureKind | null;
  skip_reason?: string | null;
  duration_ms: number;
  attempts: number;
  call_record_id?: string | null;
  message?: string | null;
}

export interface AssertionResult {
  assertion_id: string;
  kind: AssertionKind;
  severity: AssertionSeverity;
  passed: boolean;
  expected?: unknown;
  actual?: unknown;
  message?: string | null;
}

export interface NotificationEntry {
  seq: number;
  method: string;
  params: unknown;
  at: string;
}

export interface StderrLine {
  seq: number;
  line: string;
  at: string;
}

export interface Evidence {
  request: unknown;
  response_preview?: unknown;
  response_error?: CallError | null;
  truncated: boolean;
  wire: WireEntry[];
  notifications: NotificationEntry[];
  stderr: StderrLine[];
  omitted: boolean;
}

export interface TestRunCaseDetail {
  summary: TestRunCaseSummary;
  case_snapshot: unknown;
  attempt_record_ids: string[];
  assertions: AssertionResult[];
  evidence: Evidence;
}

export interface ComparisonItem {
  key: string;
  name: string;
  base_status?: TestRunCaseStatus | null;
  target_status?: TestRunCaseStatus | null;
  failure_kind?: FailureKind | null;
}

export interface CoverageDelta {
  tools: number;
  required_params: number;
  negative_categories: number;
}

export interface RunComparison {
  base_run_id: string;
  target_run_id: string;
  new_failures: ComparisonItem[];
  fixed: ComparisonItem[];
  still_failing: ComparisonItem[];
  warning_changes: ComparisonItem[];
  error_cases: ComparisonItem[];
  new_cases: ComparisonItem[];
  removed_cases: ComparisonItem[];
  duration_delta_ms: number;
  pass_rate_delta: number;
  effective_pass_rate_delta: number;
  coverage_delta: CoverageDelta;
  contract_changed: boolean;
}

export interface TestRunFilter {
  server_id?: string | null;
  suite_id?: string | null;
  status?: TestRunStatus | null;
  from?: string | null;
  to?: string | null;
}

export interface TestRunCaseFilter {
  status?: TestRunCaseStatus | null;
  failure_kind?: FailureKind | null;
  keyword?: string | null;
}

export interface Paged<T> {
  items: T[];
  total: number;
  offset: number;
  limit: number;
}

export interface ContractSnapshot {
  server_id: string;
  capability_hash: string;
  fetched_at: string;
  stale: boolean;
  tool_count: number;
  resource_count: number;
  template_count: number;
  prompt_count: number;
  snapshot: unknown;
}

export interface ContractDiff {
  changed: boolean;
  added: string[];
  removed: string[];
  modified: { kind: string; name: string; fields: string[] }[];
}

export type GenerationScope = "all" | "tools" | "resources" | "prompts" | "current";
export type GenerationProfile = "smoke" | "standard" | "thorough";

export interface GenerationOptions {
  target?: string | null;
  scope: GenerationScope;
  profile: GenerationProfile;
  include_boundary: boolean;
  include_negative: boolean;
  include_resources: boolean;
  include_prompts: boolean;
  use_history: boolean;
  allow_destructive: boolean;
  max_cases_per_tool: number;
  max_total: number;
  seed: number;
}

export interface GenerationStats {
  total: number;
  positive: number;
  boundary: number;
  negative: number;
  resources: number;
  prompts: number;
  read_only: number;
  write: number;
  destructive: number;
  unknown: number;
}

export interface GenerationPreview {
  server_id: string;
  capability_hash: string;
  capability_snapshot: unknown;
  cases: TestCaseDraft[];
  warnings: string[];
  stats: GenerationStats;
}

export interface ImportTestIssue {
  index: number;
  name?: string | null;
  reason: string;
}

export interface ImportTestCasesReport {
  added: number;
  skipped: number;
  failed: ImportTestIssue[];
}

export interface ActiveTestRun {
  run_id: string;
  server_id: string;
  total: number;
  completed: number;
  started_at: string;
}
