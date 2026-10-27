import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ServerStatusActions } from "./ServerStatusActions";
import type { ServerSummary } from "../types/models";

const invokeMock = vi.mocked(invoke);

function server(state: ServerSummary["state"]): ServerSummary {
  return {
    id: "srv-1",
    name: "Mock Server",
    transport: "stdio",
    enabled: true,
    tags: [],
    last_connected_at: null,
    state,
  };
}

function renderActions(value: ServerSummary) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ServerStatusActions server={value} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({ state: "disconnected" } as never);
});

describe("ServerStatusActions", () => {
  it("未连接时点击连接调用 connect_server", async () => {
    renderActions(server({ state: "disconnected" }));

    await userEvent.click(screen.getByRole("button", { name: "连接" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("connect_server", {
        id: "srv-1",
      }),
    );
  });

  it("降级时提供重连并调用 reconnect_server", async () => {
    renderActions(server({ state: "degraded", reason: "连续传输失败" }));

    expect(screen.getByText(/降级/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重连" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("reconnect_server", {
        id: "srv-1",
      }),
    );
  });

  it("连接中显示禁用按钮", () => {
    renderActions(server({ state: "connecting" }));
    expect(screen.getByRole("button", { name: "连接中…" })).toBeDisabled();
  });

  it("OAuth 401 提示到服务器页登录（v0.3.0）", () => {
    renderActions(
      server({
        state: "error",
        error: { kind: "oauth_required", message: "需要 OAuth 登录授权" },
      }),
    );
    expect(screen.getByText(/请到「服务器」页处理/)).toBeInTheDocument();
  });
});
