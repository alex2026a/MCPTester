import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, useLocation } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Tests } from "./Tests";
import type { ServerSummary } from "../types/models";
import type { TestCase, TestSuiteSummary } from "../types/testing";

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

const server: ServerSummary = {
  id: "srv-1",
  name: "Mock Server",
  transport: "stdio",
  enabled: true,
  tags: [],
  last_connected_at: null,
  state: { state: "disconnected" },
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
  arguments: { text: "hi" },
  expect: { outcome: "success", accept_as_warning: true },
  assertions: [],
  tags: ["positive"],
  priority: "p0",
  risk: "read_only",
  source: "schema",
  generation_note: "required text",
  enabled: true,
  order_index: 0,
  timeout_secs: null,
  retry_count: 0,
  created_at: "2026-10-21T00:00:00Z",
  updated_at: "2026-10-21T00:00:00Z",
};

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{`${location.pathname}${location.search}`}</div>;
}

function renderTests() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/tests"]}>
        <Tests />
        <LocationProbe />
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
      default:
        return Promise.resolve(null);
    }
  }) as never);
});

describe("Tests 页面", () => {
  it("展示套件与用例，并可打开编辑器", async () => {
    renderTests();

    expect(await screen.findByText("回归套件")).toBeInTheDocument();
    expect(await screen.findByText("echo · 正向基线")).toBeInTheDocument();
    expect(screen.getByText("只读")).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "echo · 正向基线" }),
    );
    expect(await screen.findByText("编辑用例")).toBeInTheDocument();
    expect(screen.getByDisplayValue("echo")).toBeInTheDocument();
  });

  it("可新建套件并调用 create_test_suite", async () => {
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "list_test_suites":
          return Promise.resolve([]);
        case "list_test_cases":
          return Promise.resolve([]);
        case "create_test_suite":
          return Promise.resolve({
            id: "suite-new",
            name: "新套件",
            description: "",
            server_id: "srv-1",
            tags: [],
            created_at: "2026-10-21T00:00:00Z",
            updated_at: "2026-10-21T00:00:00Z",
          });
        default:
          return Promise.resolve(null);
      }
    }) as never);

    renderTests();
    await userEvent.click(await screen.findByRole("button", { name: "新建套件" }));
    await userEvent.type(screen.getByPlaceholderText(/文件系统工具回归/), "新套件");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(invokeMock).toHaveBeenCalledWith("create_test_suite", {
      draft: {
        name: "新套件",
        description: "",
        server_id: "srv-1",
        tags: [],
      },
    });
  });

  it("运行完成后「查看报告」跳转到对应报告", async () => {
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
        case "get_active_test_run":
          return Promise.resolve({
            run_id: "run-9",
            server_id: "srv-1",
            total: 1,
            completed: 1,
            started_at: "2026-10-21T00:00:00Z",
          });
        default:
          return Promise.resolve(null);
      }
    }) as never);

    renderTests();
    expect(await screen.findByText("测试运行中")).toBeInTheDocument();
    await waitFor(() =>
      expect(handlers.has("test://run-finished")).toBe(true),
    );

    act(() => {
      handlers.get("test://run-finished")?.({
        payload: {
          run_id: "run-9",
          status: "completed",
          summary: {
            total: 1,
            passed: 1,
            failed: 0,
            warning: 0,
            skipped: 0,
            error: 0,
            canceled: 0,
            pass_rate: 1,
            effective_pass_rate: 1,
          },
        },
      });
    });

    await userEvent.click(
      await screen.findByRole("button", { name: "查看报告" }),
    );

    expect(screen.getByTestId("location")).toHaveTextContent(
      "/reports?run=run-9",
    );
  });
});
