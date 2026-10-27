import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { RunProgress } from "./RunProgress";

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(true);
  listenMock.mockReset();
});

describe("RunProgress", () => {
  it("展示进度并支持取消", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    listenMock.mockImplementation(((event: string, handler: (event: { payload: unknown }) => void) => {
      handlers.set(event, handler);
      return Promise.resolve(() => {});
    }) as never);

    render(<RunProgress runId="run-1" onClose={() => {}} onFinished={() => {}} />);

    act(() => {
      handlers.get("test://run-progress")?.({
        payload: {
          run_id: "run-1",
          completed: 2,
          total: 3,
          passed: 1,
          failed: 1,
          warning: 0,
          skipped: 0,
          error: 0,
          current: "echo",
        },
      });
    });

    expect(screen.getByText("2/3")).toBeInTheDocument();
    expect(screen.getByText("通过 1")).toBeInTheDocument();
    expect(screen.getByText("失败 1")).toBeInTheDocument();
    expect(screen.getByText("警告 0")).toBeInTheDocument();
    expect(screen.getByText(/当前：echo/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(invokeMock).toHaveBeenCalledWith("cancel_test_run", { runId: "run-1" });
  });

  it("完成后「查看报告」回传运行 ID", async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    listenMock.mockImplementation(((event: string, handler: (event: { payload: unknown }) => void) => {
      handlers.set(event, handler);
      return Promise.resolve(() => {});
    }) as never);
    const onFinished = vi.fn();

    render(
      <RunProgress runId="run-1" onClose={() => {}} onFinished={onFinished} />,
    );

    act(() => {
      handlers.get("test://run-finished")?.({
        payload: {
          run_id: "run-1",
          status: "completed",
          summary: {
            total: 3,
            passed: 2,
            failed: 1,
            warning: 0,
            skipped: 0,
            error: 0,
            canceled: 0,
            pass_rate: 2 / 3,
            effective_pass_rate: 2 / 3,
          },
        },
      });
    });

    await userEvent.click(
      await screen.findByRole("button", { name: "查看报告" }),
    );
    expect(onFinished).toHaveBeenCalledWith("run-1");
  });
});
