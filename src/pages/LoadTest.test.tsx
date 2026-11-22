import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LoadTest } from "./LoadTest";
import type { ServerSummary } from "../types/models";
import type { TestCase, TestSuiteSummary } from "../types/testing";
import type {
  LoadProfile,
  LoadRun,
  LoadRunSummary,
  LoadTick,
} from "../types/loadtest";

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

const server: ServerSummary = {
  id: "srv-1",
  name: "Mock Server",
  transport: "stdio",
  enabled: true,
  tags: [],
  last_connected_at: null,
  state: {
    state: "ready",
    handshake: {
      server_name: "Mock Server",
      server_version: "1.0.0",
      protocol_version: "2026-07-28",
      lifecycle: "modern",
      capabilities: {},
      instructions: null,
    },
  },
};

const suite: TestSuiteSummary = {
  id: "suite-1",
  name: "回归套件",
  server_id: "srv-1",
  server_name: "Mock Server",
  case_count: 1,
  enabled_count: 1,
  baseline_run_id: null,
  last_run_at: null,
  last_run_status: null,
};

const testCase: TestCase = {
  id: "case-1",
  suite_id: "suite-1",
  name: "echo · 正向基线",
  description: "",
  method: "tools_call",
  target: "echo",
  arguments: { text: "hello" },
  expect: { outcome: "success", accept_as_warning: true },
  assertions: [],
  tags: [],
  priority: "p0",
  risk: "read_only",
  source: "manual",
  generation_note: null,
  enabled: true,
  order_index: 0,
  timeout_secs: null,
  retry_count: 0,
  created_at: "2026-10-21T00:00:00Z",
  updated_at: "2026-10-21T00:00:00Z",
};

const profile: LoadProfile = {
  name: "回归套件 压测",
  session_mode: "per_vu",
  stages: [{ duration_secs: 60, target_vus: 10 }],
  think_time_min_ms: 0,
  think_time_max_ms: 0,
  timeout_secs: 30,
  operations: [{ case_id: "case-1", weight: 1 }],
  allow_write: false,
  allow_unknown: false,
  abort_error_rate: 0.2,
  abort_p99_ms: null,
  seed: 20261112,
};

const runSummary: LoadRunSummary = {
  id: "run-1",
  server_id: "srv-1",
  server_name: "Mock Server",
  suite_id: "suite-1",
  suite_name: "回归套件",
  profile_name: "回归套件 压测",
  status: "completed",
  stop_reason: null,
  total: 120,
  success_rate: 0.99,
  rps: 12,
  p95_ms: 80,
  p99_ms: 120,
  duration_ms: 10_000,
  max_vus: 10,
  started_at: "2026-11-13T10:00:00Z",
  finished_at: "2026-11-13T10:00:10Z",
};

const run: LoadRun = {
  id: "run-1",
  server_id: "srv-1",
  server_name: "Mock Server",
  suite_id: "suite-1",
  suite_name: "回归套件",
  profile_name: "回归套件 压测",
  status: "completed",
  stop_reason: null,
  profile,
  operations: [
    {
      case_id: "case-1",
      name: "echo · 正向基线",
      method: "tools_call",
      target: "echo",
      arguments: { text: "hello" },
      risk: "read_only",
      weight: 1,
      timeout_secs: null,
    },
  ],
  manifest: {
    app_version: "0.7.0",
    server_id: "srv-1",
    server_name: "Mock Server",
    transport: "stdio",
    protocol_version: "2026-07-28",
    lifecycle: "modern",
    capability_hash: "hash",
    capability_stale: false,
    tool_count: 1,
    resource_count: 0,
    template_count: 0,
    prompt_count: 0,
    production: false,
    risk_summary: { read_only: 1, write: 0, unknown: 0, destructive: 0 },
    started_at: "2026-11-13T10:00:00Z",
  },
  summary: {
    total: 120,
    eligible: 120,
    ok: 119,
    tool_error: 1,
    rpc_error: 0,
    timeout: 0,
    transport_error: 0,
    auth_error: 0,
    canceled: 0,
    input_required: 0,
    other_error: 0,
    success_rate: 0.991,
    error_rate: 0.009,
    duration_ms: 10_000,
    rps: 12,
    p50_ms: 30,
    p90_ms: 70,
    p95_ms: 80,
    p99_ms: 120,
    max_ms: 150,
    started_calls: 121,
    completed_calls: 120,
    inflight_at_end: 0,
    connect_failures: 0,
    result_bytes: 1000,
    max_vus: 10,
    operations: [
      {
        case_id: "case-1",
        name: "echo · 正向基线",
        method: "tools_call",
        target: "echo",
        weight: 1,
        total: 120,
        eligible: 120,
        ok: 119,
        errors: 1,
        canceled: 0,
        success_rate: 0.991,
        avg_ms: 35,
        p50_ms: 30,
        p95_ms: 80,
        p99_ms: 120,
      },
    ],
  },
  series: [
    {
      at_ms: 1000,
      stage_index: 0,
      target_vus: 10,
      active_vus: 10,
      completed_delta: 12,
      eligible_delta: 12,
      error_delta: 0,
      error_rate: 0,
      rps: 12,
      p50_ms: 30,
      p95_ms: 80,
      p99_ms: 120,
      inflight: 2,
    },
  ],
  errors: [],
  started_at: "2026-11-13T10:00:00Z",
  finished_at: "2026-11-13T10:00:10Z",
};

const tick: LoadTick = {
  run_id: "run-live",
  elapsed_ms: 1000,
  stage_index: 0,
  stage_count: 1,
  target_vus: 10,
  active_vus: 10,
  started_calls: 13,
  completed_calls: 12,
  eligible: 12,
  ok: 12,
  errors: 0,
  inflight: 1,
  rps: 12,
  p50_ms: 30,
  p95_ms: 80,
  p99_ms: 120,
  error_rate: 0,
  recent_errors: [],
};

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/loadtest"]}>
        <LoadTest />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "list_servers":
        return Promise.resolve([server]);
      case "list_test_suites":
        return Promise.resolve([suite]);
      case "list_test_cases":
        return Promise.resolve([testCase]);
      case "get_active_load_test":
        return Promise.resolve(null);
      case "list_load_runs":
        return Promise.resolve({
          items: [runSummary],
          total: 1,
          offset: 0,
          limit: 50,
        });
      case "get_load_run":
        return Promise.resolve(run);
      case "preview_load_test":
        return Promise.resolve({
          server_id: "srv-1",
          suite_id: "suite-1",
          production: false,
          risk_summary: { read_only: 1, write: 0, unknown: 0, destructive: 0 },
          operations: [
            {
              case_id: "case-1",
              name: "echo · 正向基线",
              method: "tools_call",
              target: "echo",
              weight: 1,
              effective_risk: "read_only",
              allowed: true,
              reason: null,
            },
          ],
          warnings: [],
          can_start: true,
        });
      case "start_load_test":
        return Promise.resolve("run-live");
      case "cancel_load_test":
        return Promise.resolve(true);
      default:
        return Promise.resolve(null);
    }
  }) as never);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("压测页面", () => {
  it("展示历史运行与结果详情", async () => {
    renderPage();
    expect(await screen.findByText("压测历史")).toBeInTheDocument();
    expect((await screen.findAllByText("回归套件 压测")).length).toBeGreaterThan(0);
    expect(await screen.findByText("按操作拆分")).toBeInTheDocument();
    expect(screen.getByText("echo · 正向基线")).toBeInTheDocument();
  });

  it("新建压测调用 preview 与 start_load_test", async () => {
    renderPage();
    await userEvent.click(
      await screen.findByRole("button", { name: "新建压测" }),
    );
    expect(
      await screen.findByRole("heading", { name: "新建压测" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "preview_load_test",
        expect.objectContaining({
          serverId: "srv-1",
          suiteId: "suite-1",
        }),
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "全不选" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "开始压测" })).toBeDisabled(),
    );
    await userEvent.click(screen.getByRole("button", { name: "全选" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "开始压测" })).toBeEnabled(),
    );
    await userEvent.click(screen.getByRole("button", { name: "开始压测" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "start_load_test",
        expect.objectContaining({
          serverId: "srv-1",
          suiteId: "suite-1",
          confirmProduction: false,
        }),
      ),
    );
    expect(await screen.findByText("压测运行中")).toBeInTheDocument();
  });

  it("运行结束后刷新详情缓存，不显示启动时的运行中快照", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    listenMock.mockImplementation(((
      event: string,
      handler: (event: { payload: unknown }) => void,
    ) => {
      handlers.set(event, handler);
      return Promise.resolve(() => {});
    }) as never);

    const runningSummary: LoadRunSummary = {
      ...runSummary,
      status: "running",
      total: 0,
      success_rate: 0,
      rps: 0,
      p95_ms: 0,
      p99_ms: 0,
      finished_at: null,
    };
    const runningRun: LoadRun = {
      ...run,
      status: "running",
      summary: {
        ...run.summary,
        total: 0,
        eligible: 0,
        ok: 0,
        success_rate: 0,
        error_rate: 0,
        duration_ms: 0,
        rps: 0,
        p50_ms: 0,
        p90_ms: 0,
        p95_ms: 0,
        p99_ms: 0,
        max_ms: 0,
        started_calls: 0,
        completed_calls: 0,
        operations: [],
      },
      series: [],
      finished_at: null,
    };
    const completedRun: LoadRun = {
      ...run,
      summary: {
        ...run.summary,
        total: 1514,
        eligible: 1514,
        ok: 1514,
        success_rate: 1,
        error_rate: 0,
        duration_ms: 60_000,
        rps: 25.2,
        p95_ms: 800,
        p99_ms: 900,
      },
    };
    let detailCalls = 0;
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "list_test_suites":
          return Promise.resolve([suite]);
        case "list_test_cases":
          return Promise.resolve([testCase]);
        case "get_active_load_test":
          return Promise.resolve({
            run_id: "run-1",
            server_id: "srv-1",
            suite_name: "回归套件",
            profile_name: "回归套件 压测",
            started_at: "2026-11-13T10:00:00Z",
            latest: { ...tick, run_id: "run-1" },
          });
        case "list_load_runs":
          return Promise.resolve({
            items: [runningSummary],
            total: 1,
            offset: 0,
            limit: 50,
          });
        case "get_load_run":
          detailCalls += 1;
          return Promise.resolve(detailCalls === 1 ? runningRun : completedRun);
        default:
          return Promise.resolve(null);
      }
    }) as never);

    renderPage();
    await waitFor(() => expect(detailCalls).toBeGreaterThan(0));
    await waitFor(() =>
      expect(handlers.has("loadtest://run-finished")).toBe(true),
    );
    act(() => {
      handlers.get("loadtest://run-finished")?.({
        payload: { run_id: "run-1", status: "completed", stop_reason: null },
      });
    });
    await waitFor(() => expect(detailCalls).toBeGreaterThan(1));
    expect(await screen.findByText("1514")).toBeInTheDocument();
    expect(await screen.findByText("完成")).toBeInTheDocument();
  });

  it("活动运行接收 tick 并支持停止", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    listenMock.mockImplementation(((
      event: string,
      handler: (event: { payload: unknown }) => void,
    ) => {
      handlers.set(event, handler);
      return Promise.resolve(() => {});
    }) as never);
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "list_test_suites":
          return Promise.resolve([suite]);
        case "list_test_cases":
          return Promise.resolve([testCase]);
        case "get_active_load_test":
          return Promise.resolve({
            run_id: "run-live",
            server_id: "srv-1",
            suite_name: "回归套件",
            profile_name: "回归套件 压测",
            started_at: "2026-11-13T10:00:00Z",
            latest: tick,
          });
        case "list_load_runs":
          return Promise.resolve({ items: [], total: 0, offset: 0, limit: 50 });
        default:
          return Promise.resolve(null);
      }
    }) as never);
    vi.spyOn(window, "confirm").mockReturnValue(true);

    renderPage();
    expect(await screen.findByText("压测运行中")).toBeInTheDocument();
    await waitFor(() => expect(handlers.has("loadtest://tick")).toBe(true));
    act(() => {
      handlers.get("loadtest://tick")?.({
        payload: { ...tick, rps: 20, p95_ms: 90 },
      });
    });
    expect(await screen.findByText("20.0")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "停止" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("cancel_load_test", {
        runId: "run-live",
      }),
    );
  });
});
