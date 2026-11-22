import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { RunResult } from "./RunResult";
import type {
  LoadCaseSnapshot,
  LoadOperation,
  LoadOperationSummary,
  LoadRun,
  LoadSummary,
} from "../../types/loadtest";

function caseSnapshot(
  caseId: string,
  name: string,
  weight: number,
): LoadCaseSnapshot {
  return {
    case_id: caseId,
    name,
    method: "tools_call",
    target: name,
    arguments: {},
    risk: "read_only",
    weight,
    timeout_secs: null,
  };
}

function opSummary(
  caseId: string,
  name: string,
  weight: number,
  patch: Partial<LoadOperationSummary> = {},
): LoadOperationSummary {
  return {
    case_id: caseId,
    name,
    method: "tools_call",
    target: name,
    weight,
    total: 0,
    eligible: 0,
    ok: 0,
    errors: 0,
    canceled: 0,
    success_rate: 0,
    avg_ms: 0,
    p50_ms: 0,
    p95_ms: 0,
    p99_ms: 0,
    ...patch,
  };
}

function makeRun(options: {
  operations?: LoadCaseSnapshot[];
  profileOperations?: LoadOperation[];
  summaryOperations?: LoadOperationSummary[];
  summary?: Partial<LoadSummary>;
}): LoadRun {
  const operations = options.operations ?? [];
  const profileOperations =
    options.profileOperations ??
    operations.map((operation) => ({
      case_id: operation.case_id,
      weight: operation.weight,
    }));
  const summaryOperations = options.summaryOperations ?? [];
  const now = "2026-11-14T10:00:00Z";
  return {
    id: "run-1",
    server_id: "srv-1",
    server_name: "Mock Server",
    suite_id: "suite-1",
    suite_name: "回归套件",
    profile_name: "回归套件 压测",
    status: "completed",
    stop_reason: null,
    profile: {
      name: "回归套件 压测",
      session_mode: "per_vu",
      stages: [{ duration_secs: 60, target_vus: 10 }],
      think_time_min_ms: 0,
      think_time_max_ms: 0,
      timeout_secs: 30,
      operations: profileOperations,
      allow_write: false,
      allow_unknown: false,
      abort_error_rate: 0.2,
      abort_p99_ms: null,
      seed: 20261112,
    },
    operations,
    manifest: {
      app_version: "0.7.2",
      server_id: "srv-1",
      server_name: "Mock Server",
      transport: "stdio",
      protocol_version: "2026-07-28",
      lifecycle: "modern",
      capability_hash: "hash",
      capability_stale: false,
      tool_count: 2,
      resource_count: 0,
      template_count: 0,
      prompt_count: 0,
      production: false,
      risk_summary: {
        read_only: 2,
        write: 0,
        unknown: 0,
        destructive: 0,
      },
      started_at: now,
    },
    summary: {
      total: 0,
      eligible: 0,
      ok: 0,
      tool_error: 0,
      rpc_error: 0,
      timeout: 0,
      transport_error: 0,
      auth_error: 0,
      canceled: 0,
      input_required: 0,
      other_error: 0,
      success_rate: 0,
      error_rate: 0,
      duration_ms: 10_000,
      rps: 0,
      p50_ms: 0,
      p90_ms: 0,
      p95_ms: 0,
      p99_ms: 0,
      max_ms: 0,
      started_calls: 0,
      completed_calls: 0,
      inflight_at_end: 0,
      connect_failures: 0,
      result_bytes: 0,
      max_vus: 10,
      operations: summaryOperations,
      ...options.summary,
    },
    series: [],
    errors: [],
    started_at: now,
    finished_at: now,
  };
}

function row(name: string): HTMLElement {
  const cell = screen.getByText(name);
  const tableRow = cell.closest("tr");
  if (!tableRow) throw new Error(`找不到用例行：${name}`);
  return tableRow;
}

describe("RunResult 操作占比", () => {
  it("显示配置占比、权重与实际占比", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "list_products", 700),
        caseSnapshot("c2", "list_projects", 300),
      ],
      summaryOperations: [
        opSummary("c1", "list_products", 700, {
          total: 70,
          eligible: 70,
          ok: 70,
          success_rate: 1,
          avg_ms: 10,
          p95_ms: 12,
          p99_ms: 15,
        }),
        opSummary("c2", "list_projects", 300, {
          total: 30,
          eligible: 30,
          ok: 30,
          success_rate: 1,
          avg_ms: 20,
          p95_ms: 25,
          p99_ms: 30,
        }),
      ],
      summary: {
        total: 100,
        eligible: 100,
        ok: 100,
        success_rate: 1,
        rps: 10,
        p95_ms: 20,
        p99_ms: 30,
      },
    });
    render(<RunResult run={run} />);

    const first = row("list_products");
    expect(within(first).getByText("70.0%")).toBeInTheDocument();
    expect(within(first).getByText("权重 700")).toBeInTheDocument();
    expect(within(first).getByText("实际 70.0%")).toBeInTheDocument();

    const second = row("list_projects");
    expect(within(second).getByText("30.0%")).toBeInTheDocument();
    expect(within(second).getByText("权重 300")).toBeInTheDocument();
    expect(within(second).getByText("实际 30.0%")).toBeInTheDocument();

    expect(
      within(screen.getByTestId("metric-成功率")).getByText("100.0%"),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("metric-p95")).getByText("20ms"),
    ).toBeInTheDocument();
  });

  it("实际占比按有效调用数计算，可与配置占比不同", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "list_products", 700),
        caseSnapshot("c2", "list_projects", 300),
      ],
      summaryOperations: [
        opSummary("c1", "list_products", 700, {
          total: 50,
          eligible: 50,
          ok: 50,
          success_rate: 1,
        }),
        opSummary("c2", "list_projects", 300, {
          total: 50,
          eligible: 50,
          ok: 50,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 100,
        eligible: 100,
        ok: 100,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    expect(
      within(row("list_products")).getByText("实际 50.0%"),
    ).toBeInTheDocument();
    expect(
      within(row("list_projects")).getByText("实际 50.0%"),
    ).toBeInTheDocument();
  });

  it("从未调用的操作仍出现，实际占比为 0.0%", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "called_a", 500),
        caseSnapshot("c2", "called_b", 300),
        caseSnapshot("c3", "never_called", 200),
      ],
      summaryOperations: [
        opSummary("c1", "called_a", 500, {
          total: 60,
          eligible: 60,
          ok: 60,
          success_rate: 1,
        }),
        opSummary("c2", "called_b", 300, {
          total: 40,
          eligible: 40,
          ok: 40,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 100,
        eligible: 100,
        ok: 100,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    const never = row("never_called");
    expect(within(never).getByText("20.0%")).toBeInTheDocument();
    expect(within(never).getByText("权重 200")).toBeInTheDocument();
    expect(within(never).getByText("实际 0.0%")).toBeInTheDocument();
    expect(within(never).getAllByText("-").length).toBeGreaterThanOrEqual(4);
  });

  it("旧 run 的 1/1/1 权重归一化为 33.4/33.3/33.3", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "alpha", 1),
        caseSnapshot("c2", "beta", 1),
        caseSnapshot("c3", "gamma", 1),
      ],
      summaryOperations: [
        opSummary("c1", "alpha", 1, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
        opSummary("c2", "beta", 1, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
        opSummary("c3", "gamma", 1, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 3,
        eligible: 3,
        ok: 3,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    expect(within(row("alpha")).getByText("33.4%")).toBeInTheDocument();
    expect(within(row("beta")).getByText("33.3%")).toBeInTheDocument();
    expect(within(row("gamma")).getByText("33.3%")).toBeInTheDocument();
    expect(
      within(row("alpha")).getByText("实际 33.3%"),
    ).toBeInTheDocument();
  });

  it("total=0 且 eligible=0 时成功率/延迟显示 -", () => {
    const run = makeRun({
      operations: [caseSnapshot("c1", "echo", 1000)],
      summaryOperations: [
        opSummary("c1", "echo", 1000, { total: 0, eligible: 0 }),
      ],
      summary: { total: 0, eligible: 0 },
    });
    render(<RunResult run={run} />);

    const first = row("echo");
    expect(within(first).getByText("100.0%")).toBeInTheDocument();
    expect(within(first).getByText("实际 -")).toBeInTheDocument();
    expect(within(first).getAllByText("-").length).toBeGreaterThanOrEqual(4);
    expect(
      within(screen.getByTestId("metric-成功率")).getByText("-"),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("metric-p95")).getByText("-"),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("metric-p99")).getByText("-"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("无有效调用样本（全部取消或未调用）"),
    ).toBeInTheDocument();
  });

  it("全部取消时显示取消数，成功率/延迟不当作有效样本", () => {
    const run = makeRun({
      operations: [caseSnapshot("c1", "echo", 1000)],
      summaryOperations: [
        opSummary("c1", "echo", 1000, {
          total: 100,
          eligible: 0,
          canceled: 100,
          avg_ms: 10,
          p95_ms: 12,
          p99_ms: 15,
        }),
      ],
      summary: {
        total: 100,
        eligible: 0,
        canceled: 100,
        rps: 10,
        p95_ms: 12,
        p99_ms: 15,
      },
    });
    render(<RunResult run={run} />);

    const first = row("echo");
    expect(within(first).getByText(/取消 100/)).toBeInTheDocument();
    expect(
      within(first).getByText(
        (_, element) => element?.textContent === "实际 - · 取消 100",
      ),
    ).toBeInTheDocument();
    expect(within(first).getAllByText("-").length).toBeGreaterThanOrEqual(4);
    expect(
      within(screen.getByTestId("metric-成功率")).getByText("-"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("无有效调用样本（全部取消或未调用）"),
    ).toBeInTheDocument();
  });

  it("summary-only 额外 case 不改变已有配置占比", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "list_products", 700),
        caseSnapshot("c2", "list_projects", 300),
      ],
      summaryOperations: [
        opSummary("c1", "list_products", 700, {
          total: 70,
          eligible: 70,
          ok: 70,
          success_rate: 1,
        }),
        opSummary("c2", "list_projects", 300, {
          total: 20,
          eligible: 20,
          ok: 20,
          success_rate: 1,
        }),
        opSummary("c3", "extra_case", 0, {
          total: 10,
          eligible: 10,
          ok: 10,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 100,
        eligible: 100,
        ok: 100,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    expect(
      within(row("list_products")).getByText("70.0%"),
    ).toBeInTheDocument();
    expect(
      within(row("list_projects")).getByText("30.0%"),
    ).toBeInTheDocument();
    const extra = row("extra_case");
    expect(within(extra).queryByText("0.1%")).toBeNull();
    expect(within(extra).getByText("实际 10.0%")).toBeInTheDocument();
    expect(within(extra).getAllByText("-").length).toBeGreaterThanOrEqual(2);
  });

  it("run.operations 为空时回退到 profile.operations", () => {
    const run = makeRun({
      operations: [],
      profileOperations: [
        { case_id: "c1", weight: 1 },
        { case_id: "c2", weight: 1 },
      ],
      summaryOperations: [
        opSummary("c1", "alpha", 1, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
        opSummary("c2", "beta", 1, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 2,
        eligible: 2,
        ok: 2,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    expect(within(row("alpha")).getByText("50.0%")).toBeInTheDocument();
    expect(within(row("beta")).getByText("50.0%")).toBeInTheDocument();
    expect(within(row("alpha")).getByText("实际 50.0%")).toBeInTheDocument();
  });

  it("run.operations 与 summary.operations 都为空时不崩溃", () => {
    const run = makeRun({
      operations: [],
      profileOperations: [],
      summaryOperations: [],
      summary: { total: 0, eligible: 0 },
    });
    render(<RunResult run={run} />);

    expect(screen.getByText("暂无按操作数据")).toBeInTheDocument();
  });

  it("配置总权重为 0 时配置占比显示 -", () => {
    const run = makeRun({
      operations: [caseSnapshot("c1", "zero", 0)],
      summaryOperations: [
        opSummary("c1", "zero", 0, {
          total: 1,
          eligible: 1,
          ok: 1,
          success_rate: 1,
        }),
      ],
      summary: {
        total: 1,
        eligible: 1,
        ok: 1,
        success_rate: 1,
      },
    });
    render(<RunResult run={run} />);

    const first = row("zero");
    expect(within(first).getByText("权重 0")).toBeInTheDocument();
    expect(within(first).getByText("实际 100.0%")).toBeInTheDocument();
    expect(within(first).getByText("-")).toBeInTheDocument();
  });

  it("混合取消时实际占比排除取消样本", () => {
    const run = makeRun({
      operations: [
        caseSnapshot("c1", "alpha", 500),
        caseSnapshot("c2", "beta", 500),
      ],
      summaryOperations: [
        opSummary("c1", "alpha", 500, {
          total: 80,
          eligible: 80,
          ok: 80,
          success_rate: 1,
          avg_ms: 10,
          p95_ms: 12,
          p99_ms: 15,
        }),
        opSummary("c2", "beta", 500, {
          total: 20,
          eligible: 10,
          ok: 9,
          errors: 1,
          canceled: 10,
          success_rate: 0.9,
          avg_ms: 20,
          p95_ms: 25,
          p99_ms: 30,
        }),
      ],
      summary: {
        total: 100,
        eligible: 90,
        ok: 89,
        tool_error: 1,
        canceled: 10,
        success_rate: 89 / 90,
        error_rate: 1 / 90,
        rps: 9,
        p95_ms: 20,
        p99_ms: 30,
      },
    });
    render(<RunResult run={run} />);

    expect(
      within(row("alpha")).getByText("实际 88.9%"),
    ).toBeInTheDocument();
    const beta = row("beta");
    expect(
      within(beta).getByText(
        (_, element) => element?.textContent === "实际 11.1% · 取消 10",
      ),
    ).toBeInTheDocument();
    expect(within(beta).getByText(/取消 10/)).toBeInTheDocument();
    expect(within(beta).getByText("90.0%")).toBeInTheDocument();
  });
});
