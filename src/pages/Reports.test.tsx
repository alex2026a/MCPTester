import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Reports } from "./Reports";
import type {
  Coverage,
  RunOptions,
  TestRunCaseDetail,
  TestRunCaseSummary,
  TestRunDetail,
  TestRunSummary,
} from "../types/testing";

const invokeMock = vi.mocked(invoke);
const now = "2026-10-22T00:00:00Z";

const options: RunOptions = {
  stop_on_failure: false,
  retry_count: 0,
  allow_write: false,
  allow_destructive: false,
  allow_unknown: false,
  post_case_health: false,
  seed: 1,
};

const coverage: Coverage = {
  tools_total: 1,
  tools_covered: 1,
  resources_total: 0,
  resources_covered: 0,
  prompts_total: 0,
  prompts_covered: 0,
  required_params_total: 0,
  required_params_covered: 0,
  constraints_total: 0,
  constraints_covered: 0,
  negative_categories: [],
  assertion_cases: 0,
  selected_cases: 2,
};

const runSummary: TestRunSummary = {
  id: "run-1",
  suite_id: "suite-1",
  suite_name: "套件 A",
  server_id: "s1",
  server_name: "mock",
  status: "completed",
  total: 2,
  passed: 1,
  failed: 1,
  warning: 0,
  skipped: 0,
  error: 1,
  canceled: 0,
  pass_rate: 0.5,
  effective_pass_rate: 0.5,
  duration_ms: 120,
  started_at: now,
  finished_at: now,
  baseline: true,
};

const runSummary2: TestRunSummary = {
  ...runSummary,
  id: "run-2",
  suite_name: "套件 B",
  baseline: false,
};

const fullRun: TestRunDetail["run"] = {
  id: "run-1",
  suite_id: "suite-1",
  suite_name: "套件 A",
  server_id: "s1",
  server_name: "mock",
  status: "completed",
  stop_reason: null,
  options,
  manifest: {
    app_version: "0.2.2",
    server_id: "s1",
    server_name: "mock",
    protocol_version: "2026-07-28",
    lifecycle: "modern",
    transport: "stdio",
    capability_hash: "hash",
    capability_stale: false,
    suite_hash: "suite",
    seed: 1,
    tool_count: 1,
    resource_count: 0,
    template_count: 0,
    prompt_count: 0,
    started_at: now,
    finished_at: now,
    options,
  },
  summary: {
    total: 2,
    passed: 1,
    failed: 1,
    warning: 0,
    skipped: 0,
    error: 1,
    canceled: 0,
    duration_ms: 120,
    pass_rate: 0.5,
    effective_pass_rate: 0.5,
    coverage,
  },
  baseline_run_id: null,
  started_at: now,
  finished_at: now,
};

const fullRun2: TestRunDetail["run"] = {
  ...fullRun,
  id: "run-2",
  suite_name: "套件 B",
};

const failedCase: TestRunCaseSummary = {
  id: "rc-1",
  case_id: "c-1",
  case_name: "用例 A",
  method: "tools_call",
  target: "echo",
  order_index: 0,
  status: "failed",
  failure_kind: "assertion_failed",
  skip_reason: null,
  duration_ms: 12,
  attempts: 1,
  call_record_id: "call-1",
  message: "断言失败",
};

const passedCase: TestRunCaseSummary = {
  ...failedCase,
  id: "rc-2",
  case_name: "用例 B",
  status: "passed",
  failure_kind: null,
  message: null,
};

const caseDetail: TestRunCaseDetail = {
  summary: failedCase,
  case_snapshot: {
    method: "tools_call",
    target: "echo",
    arguments: { text: "hi" },
  },
  attempt_record_ids: ["call-1"],
  assertions: [],
  evidence: {
    request: { text: "hi" },
    response_preview: { content: [{ type: "text", text: "ok" }] },
    response_error: null,
    truncated: false,
    wire: [],
    notifications: [],
    stderr: [],
    omitted: false,
  },
};

function renderReports() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Reports />
    </QueryClientProvider>,
  );
}

function lastCaseCall(): { runId: string | null; status: string | null } {
  const calls = invokeMock.mock.calls.filter(
    ([cmd]) => cmd === "list_test_run_cases",
  );
  const call = calls[calls.length - 1];
  const args = call?.[1] as
    | { runId?: string; filter?: { status?: string | null } }
    | undefined;
  return {
    runId: args?.runId ?? null,
    status: args?.filter?.status ?? null,
  };
}

function lastCaseFilter(): { status: string | null } {
  return { status: lastCaseCall().status };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string, raw?: unknown) => {
    const args = raw as
      | { runId?: string; filter?: { status?: string | null } }
      | undefined;
    if (cmd === "list_test_runs") {
      return Promise.resolve({
        items: [runSummary, runSummary2],
        total: 2,
        offset: 0,
        limit: 100,
      });
    }
    if (cmd === "get_test_run") {
      return Promise.resolve({
        run: args?.runId === "run-2" ? fullRun2 : fullRun,
        coverage,
      });
    }
    if (cmd === "list_test_run_cases") {
      const status = args?.filter?.status ?? null;
      const items =
        status === "passed"
          ? [passedCase]
          : status === "failed"
            ? [failedCase]
            : status === "skipped"
              ? []
              : [failedCase, passedCase];
      return Promise.resolve({
        items,
        total: items.length,
        offset: 0,
        limit: 500,
      });
    }
    if (cmd === "get_test_run_case") {
      return Promise.resolve(caseDetail);
    }
    return Promise.resolve(null);
  }) as never);
});

describe("Reports 页面", () => {
  it("没有运行时展示空态", async () => {
    invokeMock.mockImplementation(((cmd: string) => {
      if (cmd === "list_test_runs") {
        return Promise.resolve({ items: [], total: 0, offset: 0, limit: 100 });
      }
      return Promise.resolve(null);
    }) as never);
    renderReports();
    expect(await screen.findByText("还没有测试运行")).toBeInTheDocument();
  });

  it("点击状态卡筛选，再次点击取消", async () => {
    renderReports();
    expect(await screen.findByText("用例 A")).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "筛选：通过（1）" }),
    );
    expect(
      screen.getByRole("button", { name: "筛选：通过（1）" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByText("用例 B")).toBeInTheDocument();
    expect(screen.queryByText("用例 A")).not.toBeInTheDocument();
    expect(lastCaseFilter()).toEqual({ status: "passed" });

    await userEvent.click(
      screen.getByRole("button", { name: "筛选：通过（1）" }),
    );
    expect(await screen.findByText("用例 A")).toBeInTheDocument();
    expect(lastCaseFilter()).toEqual({ status: null });
  });

  it("全部卡清除筛选，命中数与运行总数分开", async () => {
    renderReports();
    expect(await screen.findByText("用例 A")).toBeInTheDocument();
    expect(
      screen.getByText((_, element) => element?.textContent === "2/2 条"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重跑失败（2）" })).toBeEnabled();

    await userEvent.click(
      screen.getByRole("button", { name: "筛选：通过（1）" }),
    );
    expect(
      await screen.findByText((_, element) => element?.textContent === "1/2 条"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重跑失败（2）" })).toBeEnabled();

    await userEvent.click(
      screen.getByRole("button", { name: "显示全部用例（2）" }),
    );
    expect(await screen.findByText("用例 A")).toBeInTheDocument();
    expect(lastCaseFilter()).toEqual({ status: null });
  });

  it("筛选为空时显示空态和清除入口", async () => {
    renderReports();
    expect(await screen.findByText("用例 A")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "筛选：跳过（0）" }),
    );
    expect(await screen.findByText("没有符合筛选的用例。")).toBeInTheDocument();
    await userEvent.click(
      screen.getAllByRole("button", { name: "清除筛选" })[0],
    );
    expect(await screen.findByText("用例 A")).toBeInTheDocument();
  });

  it("切换运行或运行状态时清空用例筛选", async () => {
    renderReports();
    await userEvent.click(
      await screen.findByRole("button", { name: "筛选：通过（1）" }),
    );
    expect(await screen.findByText("用例 B")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /套件 B/ }));
    await waitFor(() => expect(lastCaseCall().runId).toBe("run-2"));
    expect(lastCaseCall().status).toBeNull();

    await userEvent.selectOptions(screen.getByRole("combobox"), "failed");
    await waitFor(() => expect(lastCaseCall().runId).toBe("run-1"));
    expect(lastCaseCall().status).toBeNull();
  });

  it("用例结果加载失败时显示错误与重试，不误判为空态", async () => {
    const base = invokeMock.getMockImplementation() as
      | ((cmd: string, args?: unknown) => unknown)
      | undefined;
    invokeMock.mockImplementation(((cmd: string, args?: unknown) => {
      if (cmd === "list_test_run_cases") {
        return Promise.reject(new Error("boom"));
      }
      return base?.(cmd, args);
    }) as never);
    renderReports();
    expect(
      await screen.findByText("用例结果加载失败。"),
    ).toBeInTheDocument();
    expect(screen.queryByText("没有符合筛选的用例。")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();
  });
});
