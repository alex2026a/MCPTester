import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ProfileDialog } from "./ProfileDialog";
import type { LoadPreview, LoadProfile } from "../../types/loadtest";
import type { TestCase, TestRisk } from "../../types/testing";

const invokeMock = vi.mocked(invoke);

let currentCases: TestCase[] = [];
let riskOverrides: Record<string, TestRisk> = {};
let previewFailures = 0;
let previewDeferred: Promise<LoadPreview> | null = null;
let resolvePreview: ((value: LoadPreview) => void) | null = null;
let production = false;

function makeCase(
  id: string,
  name: string,
  risk: TestRisk = "read_only",
): TestCase {
  return {
    id,
    suite_id: "suite-1",
    name,
    description: "",
    method: "tools_call",
    target: name,
    arguments: { text: "hi" },
    expect: { outcome: "success", accept_as_warning: true },
    assertions: [],
    tags: [],
    priority: "p1",
    risk,
    source: "manual",
    generation_note: null,
    enabled: true,
    order_index: 0,
    timeout_secs: null,
    retry_count: 0,
    created_at: "2026-11-14T00:00:00Z",
    updated_at: "2026-11-14T00:00:00Z",
  };
}

function makePreview(profile: LoadProfile): LoadPreview {
  const operations = profile.operations.map((operation) => {
    const item = currentCases.find(
      (candidate) => candidate.id === operation.case_id,
    );
    const effectiveRisk =
      riskOverrides[operation.case_id] ?? item?.risk ?? "read_only";
    const allowed =
      effectiveRisk === "read_only" ||
      (effectiveRisk === "write" && profile.allow_write) ||
      (effectiveRisk === "unknown" && profile.allow_unknown);
    const reason = allowed
      ? null
      : effectiveRisk === "destructive"
        ? "destructive_not_allowed"
        : effectiveRisk === "write"
          ? "write_not_allowed"
          : effectiveRisk === "unknown"
            ? "unknown_not_allowed"
            : null;
    return {
      case_id: operation.case_id,
      name: item?.name ?? operation.case_id,
      method: "tools_call" as const,
      target: item?.target ?? "echo",
      weight: operation.weight,
      effective_risk: effectiveRisk,
      allowed,
      reason,
    };
  });
  const riskSummary = {
    read_only: 0,
    write: 0,
    unknown: 0,
    destructive: 0,
  };
  for (const operation of operations) {
    riskSummary[operation.effective_risk] += 1;
  }
  return {
    server_id: "srv-1",
    suite_id: "suite-1",
    production,
    risk_summary: riskSummary,
    operations,
    warnings: [],
    can_start: operations.every((operation) => operation.allowed),
  };
}

function profileFor(cases: TestCase[]): LoadProfile {
  return {
    name: "回归套件 压测",
    session_mode: "per_vu",
    stages: [{ duration_secs: 60, target_vus: 10 }],
    think_time_min_ms: 0,
    think_time_max_ms: 0,
    timeout_secs: 30,
    operations: cases.map((item) => ({ case_id: item.id, weight: 1000 })),
    allow_write: false,
    allow_unknown: false,
    abort_error_rate: 0.2,
    abort_p99_ms: null,
    seed: 20261112,
  };
}

function renderDialog(
  cases: TestCase[],
  options?: {
    riskOverrides?: Record<string, TestRisk>;
    production?: boolean;
  },
) {
  currentCases = cases;
  riskOverrides = options?.riskOverrides ?? {};
  production = options?.production ?? false;
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const onClose = vi.fn();
  const onStarted = vi.fn();
  const result = render(
    <QueryClientProvider client={client}>
      <ProfileDialog
        serverId="srv-1"
        suiteId="suite-1"
        suiteName="回归套件"
        cases={cases}
        onClose={onClose}
        onStarted={onStarted}
      />
    </QueryClientProvider>,
  );
  return { ...result, onClose, onStarted };
}

function startPayload():
  | { profile: LoadProfile; confirmProduction: boolean }
  | undefined {
  const call = invokeMock.mock.calls.find(
    ([command]) => command === "start_load_test",
  );
  return call?.[1] as
    | { profile: LoadProfile; confirmProduction: boolean }
    | undefined;
}

function previewCalls(): number {
  return invokeMock.mock.calls.filter(
    ([command]) => command === "preview_load_test",
  ).length;
}

function percentInput(name: string): HTMLElement {
  return screen.getByLabelText(`用例 ${name} 的占比`);
}

function weightInput(name: string): HTMLElement {
  return screen.getByLabelText(`用例 ${name} 的权重`);
}

function startButton(): HTMLElement {
  return screen.getByRole("button", { name: "开始压测" });
}

async function waitForStartEnabled() {
  await waitFor(() => expect(startButton()).toBeEnabled());
}

async function setPercent(name: string, value: string) {
  const input = percentInput(name);
  await userEvent.clear(input);
  await userEvent.type(input, value);
}

async function setWeight(name: string, value: string) {
  const input = weightInput(name);
  await userEvent.clear(input);
  await userEvent.type(input, value);
}

beforeEach(() => {
  invokeMock.mockReset();
  currentCases = [];
  riskOverrides = {};
  previewFailures = 0;
  previewDeferred = null;
  resolvePreview = null;
  production = false;
  invokeMock.mockImplementation(((command: string, params: unknown) => {
    if (command === "preview_load_test") {
      if (previewFailures > 0) {
        previewFailures -= 1;
        return Promise.reject(new Error("preview failed"));
      }
      if (previewDeferred) {
        return previewDeferred;
      }
      const { profile } = params as { profile: LoadProfile };
      return Promise.resolve(makePreview(profile));
    }
    if (command === "start_load_test") {
      return Promise.resolve("run-live");
    }
    return Promise.resolve(null);
  }) as never);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ProfileDialog 操作调用占比", () => {
  it("高级执行参数默认收起并展示当前摘要", async () => {
    const { container } = renderDialog([makeCase("case-1", "echo-1")]);
    await waitForStartEnabled();

    const details = container.querySelector("details");
    expect(details).not.toBeNull();
    expect(details?.open).toBe(false);
    expect(details?.textContent).toContain(
      "think 0–0ms · 超时 30s · 错误率 0.2",
    );
    expect(screen.getByLabelText("think time 最小（ms）")).toHaveValue(0);
  });

  it("默认均分 100.0%，启动 payload 使用 0.1% 单位", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    expect(percentInput("echo-1")).toHaveValue("50.0");
    expect(percentInput("echo-2")).toHaveValue("50.0");
    expect(screen.getByText("合计 100.0%")).toBeInTheDocument();

    await userEvent.click(startButton());
    await waitFor(() => expect(startPayload()).toBeDefined());
    expect(startPayload()?.profile.operations).toEqual([
      { case_id: "case-1", weight: 500 },
      { case_id: "case-2", weight: 500 },
    ]);
  });

  it("逐字符输入 70/30，启动权重 700/300", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    await setPercent("echo-1", "70");
    await setPercent("echo-2", "30");
    await userEvent.tab();
    await waitFor(() => expect(screen.getByText("合计 100.0%")).toBeInTheDocument());
    expect(percentInput("echo-1")).toHaveValue("70.0");
    expect(percentInput("echo-2")).toHaveValue("30.0");

    await userEvent.click(startButton());
    await waitFor(() => expect(startPayload()).toBeDefined());
    expect(startPayload()?.profile.operations).toEqual([
      { case_id: "case-1", weight: 700 },
      { case_id: "case-2", weight: 300 },
    ]);
  });

  it("支持 0.1% 精度", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    await setPercent("echo-1", "0.1");
    await setPercent("echo-2", "99.9");
    await userEvent.tab();
    await waitFor(() =>
      expect(screen.getByText("合计 100.0%")).toBeInTheDocument(),
    );
    expect(percentInput("echo-1")).toHaveValue("0.1");
    expect(percentInput("echo-2")).toHaveValue("99.9");

    await userEvent.click(startButton());
    await waitFor(() => expect(startPayload()).toBeDefined());
    expect(startPayload()?.profile.operations).toEqual([
      { case_id: "case-1", weight: 1 },
      { case_id: "case-2", weight: 999 },
    ]);
  });

  it("合计不为 100.0% 时禁用开始，归一化恢复", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    await setPercent("echo-1", "70");
    await waitFor(() =>
      expect(screen.getByText(/合计 120.0%/)).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();
    expect(
      screen.getByText(/已选用例占比合计必须为 100.0%/),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "归一化" }));
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("58.3"));
    expect(percentInput("echo-2")).toHaveValue("41.7");
    await waitForStartEnabled();
  });

  it("越界输入显示错误并禁用开始", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    await setPercent("echo-1", "100.5");
    await waitFor(() =>
      expect(
        screen.getByText(/必须在 0.1%–100.0%/),
      ).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();

    await setPercent("echo-1", "5000");
    await waitFor(() =>
      expect(
        screen.getByText(/必须在 0.1%–100.0%/),
      ).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();
  });

  it("0 与空值不合法", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();

    await userEvent.clear(percentInput("echo-1"));
    await waitFor(() =>
      expect(
        screen.getByText(/必须在 0.1%–100.0%/),
      ).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();

    await userEvent.clear(percentInput("echo-2"));
    await waitFor(() =>
      expect(screen.getByText(/不能全部为 0/)).toBeInTheDocument(),
    );
  });

  it("取消后重新勾选会恢复原输入并参与归一化", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();
    await setPercent("echo-1", "70");
    await setPercent("echo-2", "30");
    await userEvent.tab();
    await waitFor(() =>
      expect(screen.getByText("合计 100.0%")).toBeInTheDocument(),
    );

    const checkbox = screen.getByLabelText("选择用例 echo-2");
    await userEvent.click(checkbox);
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("100.0"));

    await userEvent.click(checkbox);
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("76.9"));
    expect(percentInput("echo-2")).toHaveValue("23.1");
    await waitForStartEnabled();
  });

  it("百分比与权重切换保持比例", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();
    await setPercent("echo-1", "70");
    await setPercent("echo-2", "30");
    await userEvent.tab();
    await waitFor(() =>
      expect(screen.getByText("合计 100.0%")).toBeInTheDocument(),
    );

    await userEvent.click(screen.getByRole("button", { name: "权重" }));
    expect(await screen.findByLabelText("用例 echo-1 的权重")).toHaveValue("700");
    expect(weightInput("echo-2")).toHaveValue("300");

    await userEvent.click(screen.getByRole("button", { name: "百分比" }));
    expect(await screen.findByLabelText("用例 echo-1 的占比")).toHaveValue("70.0");
    expect(percentInput("echo-2")).toHaveValue("30.0");
  });

  it("权重 1/3 切百分比为 25.0/75.0", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();
    await userEvent.click(screen.getByRole("button", { name: "权重" }));
    await screen.findByLabelText("用例 echo-1 的权重");

    await setWeight("echo-1", "1");
    await setWeight("echo-2", "3");
    await userEvent.click(screen.getByRole("button", { name: "百分比" }));
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("25.0"));
    expect(percentInput("echo-2")).toHaveValue("75.0");
  });

  it("权重合计为 0 时不能切到百分比", async () => {
    renderDialog([makeCase("case-1", "echo-1")]);
    await waitForStartEnabled();
    await userEvent.click(screen.getByRole("button", { name: "权重" }));
    await setWeight("echo-1", "0");

    expect(screen.getByRole("button", { name: "百分比" })).toBeDisabled();
  });

  it("权重含 0 时切百分比需要确认", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();
    await userEvent.click(screen.getByRole("button", { name: "权重" }));
    await screen.findByLabelText("用例 echo-1 的权重");
    await setWeight("echo-1", "0");
    await setWeight("echo-2", "1000");

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    await userEvent.click(screen.getByRole("button", { name: "百分比" }));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(weightInput("echo-1")).toBeInTheDocument();

    confirmSpy.mockReturnValue(true);
    await userEvent.click(screen.getByRole("button", { name: "百分比" }));
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("0.1"));
    expect(percentInput("echo-2")).toHaveValue("99.9");
  });

  it("权重低于 0.1% 时切百分比需要确认", async () => {
    renderDialog([makeCase("case-1", "echo-1"), makeCase("case-2", "echo-2")]);
    await waitForStartEnabled();
    await userEvent.click(screen.getByRole("button", { name: "权重" }));
    await screen.findByLabelText("用例 echo-1 的权重");
    await setWeight("echo-1", "1");
    await setWeight("echo-2", "4999");

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    await userEvent.click(screen.getByRole("button", { name: "百分比" }));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("0.1"));
    expect(percentInput("echo-2")).toHaveValue("99.9");
  });

  it("preview 升级 destructive 后排除并允许归一化启动", async () => {
    renderDialog(
      [makeCase("case-1", "echo-1"), makeCase("case-2", "delete-2")],
      { riskOverrides: { "case-2": "destructive" } },
    );
    await waitFor(() =>
      expect(screen.getByLabelText("选择用例 delete-2")).toBeDisabled(),
    );
    expect(screen.getByLabelText("用例 delete-2 的占比")).toBeDisabled();
    await waitFor(() =>
      expect(screen.getByText(/合计 50.0%/)).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "归一化" }));
    await waitFor(() => expect(percentInput("echo-1")).toHaveValue("100.0"));
    await waitForStartEnabled();
  });

  it("preview 请求中禁用开始", async () => {
    previewDeferred = new Promise<LoadPreview>((resolve) => {
      resolvePreview = resolve;
    });
    renderDialog([makeCase("case-1", "echo-1")]);

    expect(startButton()).toBeDisabled();
    resolvePreview?.(makePreview(profileFor([makeCase("case-1", "echo-1")])));
    await waitForStartEnabled();
  });

  it("preview 失败时禁用开始并提示重试", async () => {
    renderDialog([makeCase("case-1", "echo-1")]);
    await waitForStartEnabled();

    previewFailures = 1;
    await userEvent.click(
      screen.getByRole("button", { name: "重新评估风险" }),
    );
    await waitFor(() =>
      expect(screen.getByText(/风险评估失败/)).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();
  });

  it("最多选择 50 个用例", async () => {
    const many = Array.from({ length: 51 }, (_, index) =>
      makeCase(`case-${index + 1}`, `echo-${index + 1}`),
    );
    renderDialog(many);
    await waitForStartEnabled();

    expect(screen.getByText(/可用用例超过 50 个/)).toBeInTheDocument();
    expect(screen.getByLabelText("选择用例 echo-51")).toBeDisabled();
    expect(screen.getByLabelText("用例 echo-51 的占比")).toBeDisabled();

    await userEvent.click(screen.getByLabelText("选择用例 echo-1"));
    await waitFor(() =>
      expect(screen.getByLabelText("选择用例 echo-51")).toBeEnabled(),
    );
    await userEvent.click(screen.getByLabelText("选择用例 echo-51"));
    await waitFor(() =>
      expect(screen.getByLabelText("用例 echo-51 的占比")).toBeEnabled(),
    );
    await waitFor(() =>
      expect(screen.getByText("合计 100.0%")).toBeInTheDocument(),
    );
  });

  it("destructive 用例不可选且不进入启动 payload", async () => {
    renderDialog([
      makeCase("case-1", "echo-1"),
      makeCase("case-2", "delete-2", "destructive"),
    ]);
    await waitForStartEnabled();

    expect(screen.getByLabelText("选择用例 delete-2")).toBeDisabled();
    expect(screen.getByLabelText("用例 delete-2 的占比")).toBeDisabled();

    await userEvent.click(startButton());
    await waitFor(() => expect(startPayload()).toBeDefined());
    expect(startPayload()?.profile.operations).toEqual([
      { case_id: "case-1", weight: 1000 },
    ]);
  });

  it("生产 write 需允许写操作并二次确认", async () => {
    renderDialog([makeCase("case-1", "write-1", "write")], {
      production: true,
    });
    await waitFor(() =>
      expect(
        screen.getByText("我确认这是生产服务器，并已获得压测授权"),
      ).toBeInTheDocument(),
    );
    expect(startButton()).toBeDisabled();

    await userEvent.click(screen.getByLabelText("允许写操作（write）"));
    await waitFor(() => expect(startButton()).toBeDisabled());

    await userEvent.click(
      screen.getByLabelText("我确认这是生产服务器，并已获得压测授权"),
    );
    await waitForStartEnabled();
  });

  it("输入占比不会重复请求 preview，手动刷新才请求", async () => {
    renderDialog([makeCase("case-1", "echo-1")]);
    await waitForStartEnabled();
    const before = previewCalls();

    await setPercent("echo-1", "50");
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(previewCalls()).toBe(before);

    await userEvent.click(
      screen.getByRole("button", { name: "重新评估风险" }),
    );
    await waitFor(() => expect(previewCalls()).toBe(before + 1));
  });
});
