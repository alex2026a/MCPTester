import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { ResultViewer } from "./ResultViewer";
import type { CallRecord } from "../types/models";

const invokeMock = vi.mocked(invoke);

function record(partial: Partial<CallRecord>): CallRecord {
  return {
    id: "c1",
    server_id: "s1",
    server_name: "mock",
    method: "tools_call",
    target: "echo",
    params: {},
    result: { content: [{ type: "text", text: "echo: hi" }] },
    error: null,
    status: "ok",
    duration_ms: 12,
    protocol_version: "2026-07-28",
    transport: "stdio",
    truncated: false,
    size_bytes: 10,
    origin: "manual",
    created_at: "2026-10-18T00:00:00Z",
    ...partial,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
});

describe("ResultViewer", () => {
  it("展示状态/耗时，并默认渲染 MCP 文本内容", () => {
    render(<ResultViewer record={record({})} />);
    expect(screen.getByText("成功")).toBeInTheDocument();
    expect(screen.getByText(/耗时 12ms/)).toBeInTheDocument();
    // 渲染页签：content[0].text 直接展示（无需在 JSON 里找）
    expect(screen.getByText("echo: hi")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "渲染" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "结构" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "原文" })).toBeInTheDocument();
  });

  it("可切换到结构与原文视图", async () => {
    render(<ResultViewer record={record({})} />);
    await userEvent.click(screen.getByRole("button", { name: "结构" }));
    expect(await screen.findByText("content")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "原文" }));
    expect(await screen.findByText(/"echo: hi"/)).toBeInTheDocument();
  });

  it("错误结果高亮并显示错误消息（仅一次）", () => {
    render(
      <ResultViewer
        record={record({
          status: "error",
          result: null,
          error: { kind: "timeout", message: "调用超时（1s）" },
        })}
      />,
    );
    expect(screen.getByText("失败")).toBeInTheDocument();
    expect(screen.getByText("timeout")).toBeInTheDocument();
    expect(screen.getAllByText("调用超时（1s）")).toHaveLength(1);
  });

  it("截断结果提供完整内容入口", async () => {
    invokeMock.mockImplementation((async (cmd: string) => {
      if (cmd === "read_full_result") return JSON.stringify({ content: [{ type: "text", text: "full" }] });
      return null;
    }) as typeof invoke);

    render(
      <ResultViewer
        record={record({ truncated: true, size_bytes: 2_000_000 })}
      />,
    );
    expect(screen.getByText("结果已截断")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "打开完整结果" }));
    expect(await screen.findByText("完整结果")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("read_full_result", { recordId: "c1" });
  });
});
