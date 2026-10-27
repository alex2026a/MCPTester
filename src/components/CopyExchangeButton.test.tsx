import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { ResultViewer } from "./ResultViewer";
import type { CallRecord, ServerConfig } from "../types/models";

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

function httpServer(): ServerConfig {
  return {
    id: "s1",
    name: "mock",
    transport: "streamable_http",
    stdio: null,
    http: {
      url: "https://example.com/mcp?token=url-secret",
      headers: [
        { key: "Authorization", value: "Bearer real-token", secret: false },
        { key: "X-Trace", value: "trace-1", secret: false },
      ],
      timeout_secs: null,
      allow_insecure_tls: false,
      oauth: null,
    },
    protocol_version_pin: null,
    enabled: true,
    tags: [],
    origin: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    last_connected_at: null,
  };
}

function copiedText(): string {
  const call = writeText.mock.calls[0];
  if (!call) throw new Error("没有调用剪贴板");
  return call[0];
}

async function openMenu() {
  await userEvent.click(screen.getByRole("button", { name: "更多复制选项" }));
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

describe("ResultViewer 复制菜单", () => {
  it("主按钮默认复制响应 body", async () => {
    render(<ResultViewer record={record()} />);
    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toEqual({
      content: [{ type: "text", text: "echo: hi" }],
    });
    expect(screen.getByRole("button", { name: "已复制" })).toBeInTheDocument();
  });

  it("菜单可复制完整 JSON-RPC 请求", async () => {
    render(<ResultViewer record={record()} />);
    await openMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "复制请求" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toMatchObject({
      jsonrpc: "2.0",
      id: "c1",
      method: "tools/call",
      params: { name: "echo", arguments: { text: "hi" } },
    });
  });

  it("菜单可复制请求和响应 Markdown", async () => {
    render(<ResultViewer record={record()} />);
    await openMenu();
    await userEvent.click(
      screen.getByRole("menuitem", { name: "复制请求和响应" }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(copiedText()).toContain("**请求**");
    expect(copiedText()).toContain("**响应**");
    expect(copiedText()).toContain('"method": "tools/call"');
  });

  it("截断结果先读取完整文件再复制", async () => {
    const full = { content: [{ type: "text", text: "full" }] };
    invokeMock.mockImplementation((async (command: string) => {
      if (command === "read_full_result") return JSON.stringify(full);
      return null;
    }) as typeof invoke);

    render(
      <ResultViewer
        record={record({
          truncated: true,
          result: null,
          size_bytes: 2_000_000,
        })}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("read_full_result", {
        recordId: "c1",
      }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toEqual(full);
  });

  it("10MB+ 截断结果取消确认后不读文件也不写剪贴板", async () => {
    const full = { content: [{ type: "text", text: "full" }] };
    invokeMock.mockImplementation((async (command: string) => {
      if (command === "read_full_result") return JSON.stringify(full);
      return null;
    }) as typeof invoke);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(
      <ResultViewer
        record={record({
          truncated: true,
          result: null,
          size_bytes: 12 * 1024 * 1024,
        })}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(invokeMock).not.toHaveBeenCalledWith("read_full_result", {
      recordId: "c1",
    });
    expect(writeText).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("read_full_result", {
        recordId: "c1",
      }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toEqual(full);
  });

  it("菜单提示敏感字段，且不误报 max_tokens / progressToken", async () => {
    render(
      <ResultViewer
        record={record({
          params: {
            name: "echo",
            arguments: {
              token: "abc",
              max_tokens: 100,
              progressToken: 7,
            },
          },
        })}
      />,
    );
    await openMenu();
    expect(screen.getByText(/参数包含 1 个敏感字段/)).toBeInTheDocument();
  });

  it("isError=true 时复制的是 result body", async () => {
    render(
      <ResultViewer
        record={record({
          status: "error",
          result: { content: [{ type: "text", text: "boom" }], isError: true },
          error: {
            kind: "rpc_error",
            code: 0,
            message: "工具返回 isError=true",
            data: { isError: true },
          },
        })}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "复制响应" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toMatchObject({ isError: true });
  });

  it("取消记录禁用响应复制，但仍可复制成对内容", async () => {
    render(
      <ResultViewer
        record={record({ status: "canceled", result: null, error: null })}
      />,
    );
    expect(screen.getByRole("button", { name: "复制响应" })).toBeDisabled();

    await openMenu();
    expect(screen.getByRole("menuitem", { name: "复制响应" })).toBeDisabled();
    const pair = screen.getByRole("menuitem", { name: "复制请求和响应" });
    expect(pair).toBeEnabled();
    await userEvent.click(pair);
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(copiedText()).toContain("调用已取消，无响应。");
  });

  it("探测记录禁用请求复制，但可复制探测结果", async () => {
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
    await openMenu();
    expect(screen.getByRole("menuitem", { name: "复制请求" })).toBeDisabled();
    const response = screen.getByRole("menuitem", { name: "复制响应" });
    expect(response).toBeEnabled();
    await userEvent.click(response);
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText())).toMatchObject({ ok: true });
  });

  it("复制时脱敏替换 token，但保留 max_tokens", async () => {
    render(
      <ResultViewer
        record={record({
          params: {
            name: "echo",
            arguments: { token: "abc", max_tokens: 100 },
          },
        })}
      />,
    );
    await openMenu();
    await userEvent.click(
      screen.getByRole("menuitemcheckbox", { name: "复制时脱敏" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "复制请求" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(copiedText()).params.arguments).toEqual({
      token: "****",
      max_tokens: 100,
    });
  });

  it("Streamable HTTP 可复制脱敏请求骨架，stdio 禁用", async () => {
    invokeMock.mockImplementation((async (command: string) => {
      if (command === "get_server") return httpServer();
      return null;
    }) as typeof invoke);

    render(
      <ResultViewer
        record={record({
          transport: "streamable_http",
          protocol_version: "2026-07-28",
        })}
      />,
    );
    await openMenu();
    await userEvent.click(
      screen.getByRole("menuitem", { name: "复制 HTTP 请求骨架" }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const text = copiedText();
    expect(text).toContain("Authorization: ****");
    expect(text).toContain("MCP-Protocol-Version: 2026-07-28");
    expect(text).not.toContain("real-token");
    expect(text).not.toContain("url-secret");
  });

  it("文本内容跳过图片 base64", async () => {
    const base64 = "A".repeat(4000);
    render(
      <ResultViewer
        record={record({
          result: {
            content: [
              { type: "text", text: "hello" },
              { type: "image", mimeType: "image/png", data: base64 },
            ],
          },
        })}
      />,
    );
    await openMenu();
    await userEvent.click(
      screen.getByRole("menuitem", { name: "复制文本内容" }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(copiedText()).toContain("hello");
    expect(copiedText()).toContain("base64 omitted");
    expect(copiedText()).not.toContain(base64);
  });

  it("stdio 禁用 HTTP 请求骨架", async () => {
    render(<ResultViewer record={record()} />);
    await openMenu();
    expect(
      screen.getByRole("menuitem", { name: "复制 HTTP 请求骨架" }),
    ).toBeDisabled();
  });
});
