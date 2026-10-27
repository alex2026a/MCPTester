import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  RecentParamsButton,
  type RecentParamsContext,
} from "./RecentParamsButton";
import type { RecentCallParam } from "../types/models";

const invokeMock = vi.mocked(invoke);

const context: RecentParamsContext = {
  serverId: "srv-1",
  method: "tools_call",
  target: "echo",
};

const item: RecentCallParam = {
  id: "call-1",
  created_at: "2026-10-26T10:32:05Z",
  status: "ok",
  duration_ms: 320,
  origin: "manual",
  preview: 'text="hello"',
  param_count: 1,
  repeat_count: 3,
  params_bytes: 24,
};

function renderButton(onApply = vi.fn()) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <RecentParamsButton context={context} onApply={onApply} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return onApply;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "list_recent_call_params":
        return Promise.resolve([item]);
      case "get_call_params":
        return Promise.resolve({
          name: "echo",
          arguments: { text: "hello" },
        });
      default:
        return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
    }
  }) as typeof invoke);
});

describe("RecentParamsButton", () => {
  it("打开时按当前接口查询，默认不含测试调用", async () => {
    renderButton();
    await userEvent.click(screen.getByRole("button", { name: /历史参数/ }));

    expect(await screen.findByText('text="hello"')).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("list_recent_call_params", {
      serverId: "srv-1",
      method: "tools_call",
      target: "echo",
      limit: 20,
      includeTest: false,
    });
  });

  it("点击条目只读取参数并回调，不发送调用", async () => {
    const onApply = renderButton();
    await userEvent.click(screen.getByRole("button", { name: /历史参数/ }));
    await userEvent.click(await screen.findByText('text="hello"'));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_call_params", {
        recordId: "call-1",
      }),
    );
    expect(onApply).toHaveBeenCalledWith(
      { text: "hello" },
      expect.objectContaining({ id: "call-1" }),
      context,
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "call_tool",
      expect.anything(),
    );
  });

  it("勾选包含测试调用后重新查询", async () => {
    renderButton();
    await userEvent.click(screen.getByRole("button", { name: /历史参数/ }));
    await screen.findByText('text="hello"');
    await userEvent.click(screen.getByLabelText("包含测试调用"));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "list_recent_call_params",
        expect.objectContaining({ includeTest: true }),
      ),
    );
  });

  it("关闭浮层后在途请求不会回填", async () => {
    let resolveParams:
      | ((value: Record<string, unknown>) => void)
      | undefined;
    invokeMock.mockImplementation(((cmd: string) => {
      if (cmd === "list_recent_call_params") return Promise.resolve([item]);
      if (cmd === "get_call_params") {
        return new Promise<Record<string, unknown>>((resolve) => {
          resolveParams = resolve;
        });
      }
      return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
    }) as typeof invoke);

    const onApply = renderButton();
    await userEvent.click(screen.getByRole("button", { name: /历史参数/ }));
    await userEvent.click(await screen.findByText('text="hello"'));
    await userEvent.keyboard("{Escape}");
    resolveParams?.({ name: "echo", arguments: { text: "hello" } });

    await waitFor(() => expect(onApply).not.toHaveBeenCalled());
  });
});
