import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { ResultViewer } from "./ResultViewer";
import type { CallRecord } from "../types/models";

const invokeMock = vi.mocked(invoke);
const writeText = vi.fn(async (text: string): Promise<void> => {
  void text;
});

function record(partial: Partial<CallRecord> = {}): CallRecord {
  return {
    id: "c1",
    server_id: "s1",
    server_name: "mock",
    method: "tools_call",
    target: "echo",
    params: { name: "echo", arguments: { text: "hi" } },
    result: { content: [{ type: "text", text: "echo: hi" }] },
    error: null,
    status: "ok",
    duration_ms: 12,
    protocol_version: "2026-07-28",
    transport: "stdio",
    truncated: false,
    size_bytes: 48,
    origin: "manual",
    created_at: "2026-10-18T00:00:00Z",
    ...partial,
  };
}

beforeEach(() => {
  writeText.mockClear();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ResultViewer 请求/响应页签", () => {
  it("MRTR 记录默认最后一轮，可切到中间轮", async () => {
    const recordWithRounds = record({
      rounds: {
        rounds: [
          {
            round: 1,
            rpc_id: 1,
            request: {
              jsonrpc: "2.0",
              id: 1,
              method: "tools/call",
              params: { name: "echo", arguments: { text: "hi" } },
            },
            response: {
              jsonrpc: "2.0",
              id: 1,
              result: {
                resultType: "input_required",
                inputRequests: { name: { method: "elicitation/create" } },
              },
            },
            input_requests: { name: { method: "elicitation/create" } },
            input_responses: {
              name: { action: "accept", content: { name: "Ada" } },
            },
            truncated: false,
          },
          {
            round: 2,
            rpc_id: 2,
            request: {
              jsonrpc: "2.0",
              id: 2,
              method: "tools/call",
              params: { name: "echo", arguments: { text: "hi" } },
            },
            response: {
              jsonrpc: "2.0",
              id: 2,
              result: { content: [{ type: "text", text: "echo: hi" }] },
            },
            truncated: false,
          },
        ],
        truncated: false,
      },
    });
    render(<ResultViewer record={recordWithRounds} />);

    const selector = screen.getByRole("combobox", { name: "轮次" });
    expect(selector).toHaveValue("1");
    expect(screen.getByText("echo: hi")).toBeInTheDocument();

    await userEvent.selectOptions(selector, "0");
    expect(await screen.findByText(/该轮为等待输入/)).toBeInTheDocument();
    expect(screen.getByText("原始响应帧")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(writeText.mock.calls[0][0])).toMatchObject({
      result: { resultType: "input_required" },
    });

    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    expect(
      screen.getByRole("button", { name: "复制请求" }),
    ).toBeEnabled();
  });

  it("旧世代服务器输入有独立页签", async () => {
    render(
      <ResultViewer
        record={record({
          rounds: {
            rounds: [
              {
                round: 1,
                rpc_id: 1,
                request: {
                  jsonrpc: "2.0",
                  id: 1,
                  method: "tools/call",
                  params: { name: "echo", arguments: {} },
                },
                response: { jsonrpc: "2.0", id: 1, result: {} },
                truncated: false,
              },
            ],
            server_inputs: [
              {
                kind: "elicitation",
                request: {
                  method: "elicitation/create",
                  params: { message: "请输入测试值" },
                },
                response: { action: "accept", content: { value: "ok" } },
                at: "2026-10-29T00:00:00Z",
              },
            ],
            truncated: false,
          },
        })}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "输入（1）" }));
    expect(await screen.findByText("旧世代输入 #1")).toBeInTheDocument();
    expect(screen.getAllByText("elicitation").length).toBeGreaterThan(0);
  });

  it("默认响应页，可切换到请求页", async () => {
    render(<ResultViewer record={record()} />);
    expect(screen.getByRole("button", { name: "响应" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    expect(screen.getByRole("button", { name: "请求" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(await screen.findByText("params")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "复制请求" })).toBeEnabled();
  });

  it("探测记录禁用请求页", () => {
    render(
      <ResultViewer
        record={record({
          method: "probe",
          target: "probe",
          params: {},
          result: { ok: true, rtt_ms: 1, detail: "ok" },
        })}
      />,
    );
    expect(screen.getByRole("button", { name: "请求" })).toBeDisabled();
  });

  it("取消记录请求页可用，响应页显示无结果", async () => {
    render(
      <ResultViewer
        record={record({ status: "canceled", result: null, error: null })}
      />,
    );
    expect(screen.getByText("调用已取消，无结果。")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    expect(screen.getByRole("button", { name: "复制请求" })).toBeEnabled();
  });

  it("请求页主按钮复制请求", async () => {
    render(<ResultViewer record={record()} />);
    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    await userEvent.click(screen.getByRole("button", { name: "复制请求" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const copied = writeText.mock.calls[0]?.[0];
    expect(copied).toBeDefined();
    expect(JSON.parse(copied ?? "null")).toMatchObject({
      jsonrpc: "2.0",
      id: "c1",
      method: "tools/call",
    });
  });

  it("有原始帧时显示原始帧并可查看原始响应帧", async () => {
    render(
      <ResultViewer
        record={record({
          frames: {
            rpc_id: 3,
            request: {
              jsonrpc: "2.0",
              id: 3,
              method: "tools/call",
              params: { name: "echo" },
            },
            response: {
              jsonrpc: "2.0",
              id: 3,
              result: { content: [{ type: "text", text: "echo: hi" }] },
            },
            truncated: false,
          },
        })}
      />,
    );
    expect(screen.getByText("原始帧")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    await userEvent.click(screen.getByRole("button", { name: "复制请求" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const request = JSON.parse(writeText.mock.calls[0]?.[0] ?? "null");
    expect(request.id).toBe(3);
    expect(request.method).toBe("tools/call");

    await userEvent.click(screen.getByRole("button", { name: "响应" }));
    await userEvent.click(screen.getByRole("button", { name: "原始响应帧" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "原始响应帧" }),
    ).toBeInTheDocument();
  });

  it("原始帧超限/缺失时显示部分原始或重建", () => {
    const { rerender } = render(
      <ResultViewer
        record={record({
          frames: {
            rpc_id: 3,
            request: null,
            response: null,
            truncated: true,
          },
        })}
      />,
    );
    expect(screen.getByText("部分原始")).toBeInTheDocument();

    rerender(<ResultViewer record={record({ id: "c2" })} />);
    expect(screen.getByText("按记录重建")).toBeInTheDocument();
  });

  it("切换记录后回到响应页", async () => {
    const { rerender } = render(<ResultViewer record={record()} />);
    await userEvent.click(screen.getByRole("button", { name: "请求" }));
    expect(screen.getByRole("button", { name: "请求" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    rerender(
      <ResultViewer
        record={record({
          id: "c2",
          result: { content: [{ type: "text", text: "second" }] },
        })}
      />,
    );
    expect(screen.getByRole("button", { name: "响应" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByText("second")).toBeInTheDocument();
  });
});
