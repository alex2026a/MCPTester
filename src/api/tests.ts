import { invoke } from "@tauri-apps/api/core";
import type {
  ActiveTestRun,
  ContractDiff,
  ContractSnapshot,
  GenerationOptions,
  GenerationPreview,
  ImportTestCasesReport,
  Paged,
  RunComparison,
  RunOptions,
  TestCase,
  TestCaseDraft,
  TestRunCaseDetail,
  TestRunCaseFilter,
  TestRunCaseSummary,
  TestRunDetail,
  TestRunFilter,
  TestRunSummary,
  TestSuite,
  TestSuiteDraft,
  TestSuiteSummary,
  TestSuiteUpdateDraft,
} from "../types/testing";

export function listTestSuites(serverId?: string | null): Promise<TestSuiteSummary[]> {
  return invoke<TestSuiteSummary[]>("list_test_suites", {
    serverId: serverId ?? null,
  });
}

export function getTestSuite(id: string): Promise<TestSuite> {
  return invoke<TestSuite>("get_test_suite", { id });
}

export function createTestSuite(draft: TestSuiteDraft): Promise<TestSuite> {
  return invoke<TestSuite>("create_test_suite", { draft });
}

export function updateTestSuite(
  id: string,
  draft: TestSuiteUpdateDraft,
): Promise<TestSuite> {
  return invoke<TestSuite>("update_test_suite", { id, draft });
}

export function rebindTestSuite(id: string, serverId: string): Promise<TestSuite> {
  return invoke<TestSuite>("rebind_test_suite", { id, serverId });
}

export function deleteTestSuite(id: string): Promise<number> {
  return invoke<number>("delete_test_suite", { id });
}

export function listTestCases(suiteId: string): Promise<TestCase[]> {
  return invoke<TestCase[]>("list_test_cases", { suiteId });
}

export function createTestCases(
  suiteId: string,
  cases: TestCaseDraft[],
): Promise<TestCase[]> {
  return invoke<TestCase[]>("create_test_cases", { suiteId, cases });
}

export function updateTestCase(
  id: string,
  draft: TestCaseDraft,
): Promise<TestCase> {
  return invoke<TestCase>("update_test_case", { id, draft });
}

export function deleteTestCases(ids: string[]): Promise<number> {
  return invoke<number>("delete_test_cases", { ids });
}

export function duplicateTestCase(id: string): Promise<TestCase> {
  return invoke<TestCase>("duplicate_test_case", { id });
}

export function reorderTestCases(suiteId: string, ids: string[]): Promise<void> {
  return invoke<void>("reorder_test_cases", { suiteId, ids });
}

/** 生成用例预览（不落库）。 */
export function generateTestCases(
  serverId: string,
  options: GenerationOptions,
): Promise<GenerationPreview> {
  return invoke<GenerationPreview>("generate_test_cases", {
    serverId,
    options,
  });
}

/** 保存生成预览：原子写入用例 + 能力快照/指纹。 */
export function saveGeneratedCases(
  suiteId: string,
  cases: TestCaseDraft[],
  capabilityHash: string,
  capabilitySnapshot: unknown,
): Promise<TestCase[]> {
  return invoke<TestCase[]>("save_generated_cases", {
    suiteId,
    cases,
    capabilityHash,
    capabilitySnapshot,
  });
}

/** 运行套件（后台执行，返回 run_id）。 */
export function runTestSuite(
  suiteId: string,
  options: RunOptions,
): Promise<string> {
  return invoke<string>("run_test_suite", { suiteId, options });
}

/** 运行指定用例（必须属于同一服务器）。 */
export function runTestCases(
  serverId: string,
  caseIds: string[],
  options: RunOptions,
): Promise<string> {
  return invoke<string>("run_test_cases", { serverId, caseIds, options });
}

export function cancelTestRun(runId: string): Promise<boolean> {
  return invoke<boolean>("cancel_test_run", { runId });
}

export function getActiveTestRun(): Promise<ActiveTestRun | null> {
  return invoke<ActiveTestRun | null>("get_active_test_run");
}

export function listTestRuns(
  filter: TestRunFilter,
  offset: number,
  limit: number,
): Promise<Paged<TestRunSummary>> {
  return invoke<Paged<TestRunSummary>>("list_test_runs", {
    filter,
    offset,
    limit,
  });
}

export function getTestRun(runId: string): Promise<TestRunDetail> {
  return invoke<TestRunDetail>("get_test_run", { runId });
}

export function listTestRunCases(
  runId: string,
  filter: TestRunCaseFilter,
  offset: number,
  limit: number,
): Promise<Paged<TestRunCaseSummary>> {
  return invoke<Paged<TestRunCaseSummary>>("list_test_run_cases", {
    runId,
    filter,
    offset,
    limit,
  });
}

export function getTestRunCase(runCaseId: string): Promise<TestRunCaseDetail> {
  return invoke<TestRunCaseDetail>("get_test_run_case", { runCaseId });
}

export function setTestBaseline(runId: string): Promise<void> {
  return invoke<void>("set_test_baseline", { runId });
}

export function compareTestRuns(
  baseRunId: string,
  targetRunId: string,
): Promise<RunComparison> {
  return invoke<RunComparison>("compare_test_runs", {
    baseRunId,
    targetRunId,
  });
}

export function rerunFailedTestCases(
  runId: string,
  options: RunOptions,
): Promise<string> {
  return invoke<string>("rerun_failed_test_cases", { runId, options });
}

export function exportTestReport(
  runId: string,
  format: "html" | "json" | "junit" | "md",
  path?: string,
  includeFull = false,
  includeSensitive = false,
): Promise<string> {
  return invoke<string>("export_test_report", {
    runId,
    format,
    path: path ?? null,
    includeFull,
    includeSensitive,
  });
}

export function exportLlmPrompt(
  serverId: string,
  options: GenerationOptions,
): Promise<string> {
  return invoke<string>("export_llm_prompt", { serverId, options });
}

export function importLlmTestCases(
  suiteId: string,
  json: string,
): Promise<ImportTestCasesReport> {
  return invoke<ImportTestCasesReport>("import_llm_test_cases", {
    suiteId,
    json,
  });
}

export function getContractSnapshot(serverId: string): Promise<ContractSnapshot> {
  return invoke<ContractSnapshot>("get_contract_snapshot", { serverId });
}

export function diffContract(suiteId: string): Promise<ContractDiff> {
  return invoke<ContractDiff>("diff_contract", { suiteId });
}

export function saveCallAsCase(
  recordId: string,
  suiteId: string,
  name?: string,
): Promise<TestCase> {
  return invoke<TestCase>("save_call_as_case", {
    recordId,
    suiteId,
    name: name ?? null,
  });
}

/** 导出套件 JSON；path 为空时仅返回文本。 */
export function exportTestSuite(
  suiteId: string,
  path?: string,
): Promise<string> {
  return invoke<string>("export_test_suite", {
    suiteId,
    path: path ?? null,
  });
}

export function importTestCases(
  suiteId: string,
  json: string,
  source?: "manual" | "llm" | "history" | "schema" | "import",
): Promise<ImportTestCasesReport> {
  return invoke<ImportTestCasesReport>("import_test_cases", {
    suiteId,
    json,
    source: source ?? null,
  });
}
