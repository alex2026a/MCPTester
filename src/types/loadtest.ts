/** v0.7.0 压测领域类型（与 src-tauri/src/loadtest/models.rs 手工同步）。 */

import type { TestMethod, TestRisk } from "./testing";

export type LoadSessionMode = "per_vu" | "shared";
export type LoadCallStatus =
  | "ok"
  | "tool_error"
  | "rpc_error"
  | "timeout"
  | "transport_error"
  | "auth_error"
  | "canceled"
  | "input_required"
  | "other_error";
export type LoadRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "canceled"
  | "interrupted"
  | "aborted";

export interface LoadStage {
  duration_secs: number;
  target_vus: number;
}

export interface LoadOperation {
  case_id: string;
  weight: number;
}

export interface LoadProfile {
  name: string;
  session_mode: LoadSessionMode;
  stages: LoadStage[];
  think_time_min_ms: number;
  think_time_max_ms: number;
  timeout_secs: number;
  operations: LoadOperation[];
  allow_write: boolean;
  allow_unknown: boolean;
  abort_error_rate: number | null;
  abort_p99_ms: number | null;
  seed: number;
}

export interface LoadCaseSnapshot {
  case_id: string;
  name: string;
  method: TestMethod;
  target: string;
  arguments: unknown;
  risk: TestRisk;
  weight: number;
  timeout_secs?: number | null;
}

export interface LoadOperationSummary {
  case_id: string;
  name: string;
  method?: TestMethod | null;
  target: string;
  weight: number;
  total: number;
  eligible: number;
  ok: number;
  errors: number;
  canceled: number;
  success_rate: number;
  avg_ms: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
}

export interface LoadSummary {
  total: number;
  eligible: number;
  ok: number;
  tool_error: number;
  rpc_error: number;
  timeout: number;
  transport_error: number;
  auth_error: number;
  canceled: number;
  input_required: number;
  other_error: number;
  success_rate: number;
  error_rate: number;
  duration_ms: number;
  rps: number;
  p50_ms: number;
  p90_ms: number;
  p95_ms: number;
  p99_ms: number;
  max_ms: number;
  started_calls: number;
  completed_calls: number;
  inflight_at_end: number;
  connect_failures: number;
  result_bytes: number;
  max_vus: number;
  operations: LoadOperationSummary[];
}

export interface LoadSeriesPoint {
  at_ms: number;
  stage_index: number;
  target_vus: number;
  active_vus: number;
  completed_delta: number;
  eligible_delta: number;
  error_delta: number;
  error_rate: number;
  rps: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  inflight: number;
}

export interface LoadErrorSample {
  at: string;
  case_name: string;
  method: TestMethod;
  target: string;
  status: LoadCallStatus;
  duration_ms: number;
  message: string;
}

export interface LoadRiskSummary {
  read_only: number;
  write: number;
  unknown: number;
  destructive: number;
}

export interface LoadManifest {
  app_version: string;
  server_id: string;
  server_name: string;
  transport: string;
  protocol_version: string;
  lifecycle: string;
  capability_hash: string;
  capability_stale: boolean;
  tool_count: number;
  resource_count: number;
  template_count: number;
  prompt_count: number;
  production: boolean;
  risk_summary: LoadRiskSummary;
  started_at: string;
}

export interface LoadRun {
  id: string;
  server_id: string;
  server_name: string;
  suite_id: string;
  suite_name: string;
  profile_name: string;
  status: LoadRunStatus;
  stop_reason?: string | null;
  profile: LoadProfile;
  operations: LoadCaseSnapshot[];
  manifest: LoadManifest;
  summary: LoadSummary;
  series: LoadSeriesPoint[];
  errors: LoadErrorSample[];
  started_at: string;
  finished_at?: string | null;
}

export interface LoadRunSummary {
  id: string;
  server_id: string;
  server_name: string;
  suite_id: string;
  suite_name: string;
  profile_name: string;
  status: LoadRunStatus;
  stop_reason?: string | null;
  total: number;
  success_rate: number;
  rps: number;
  p95_ms: number;
  p99_ms: number;
  duration_ms: number;
  max_vus: number;
  started_at: string;
  finished_at?: string | null;
}

export interface LoadTick {
  run_id: string;
  elapsed_ms: number;
  stage_index: number;
  stage_count: number;
  target_vus: number;
  active_vus: number;
  started_calls: number;
  completed_calls: number;
  eligible: number;
  ok: number;
  errors: number;
  inflight: number;
  rps: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  error_rate: number;
  recent_errors: LoadErrorSample[];
}

export interface ActiveLoadRun {
  run_id: string;
  server_id: string;
  suite_name: string;
  profile_name: string;
  started_at: string;
  latest?: LoadTick | null;
}

export interface LoadPreviewOperation {
  case_id: string;
  name: string;
  method: TestMethod;
  target: string;
  weight: number;
  effective_risk: TestRisk;
  allowed: boolean;
  reason?: string | null;
}

export interface LoadPreview {
  server_id: string;
  suite_id: string;
  production: boolean;
  risk_summary: LoadRiskSummary;
  operations: LoadPreviewOperation[];
  warnings: string[];
  can_start: boolean;
}
