import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Servers } from "./Servers";
import type { ConnectionState, ServerSummary } from "../types/models";

const invokeMock = vi.mocked(invoke);

const ready: ConnectionState = {
  state: "ready",
  handshake: {
    server_name: "mock",
    server_version: "1.0.0",
    protocol_version: "2025-06-18",
    lifecycle: "legacy",
    capabilities: {},
    instructions: null,
  },
};

const alpha: ServerSummary = {
  id: "alpha",
  name: "Alpha",
  transport: "stdio",
  enabled: true,
  tags: [],
  last_connected_at: null,
  state: ready,
};

const beta: ServerSummary = {
  ...alpha,
  id: "beta",
  name: "Beta",
  state: { state: "disconnected" },
};

function renderServers() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Servers />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "list_servers":
        return Promise.resolve([alpha, beta]);
      case "get_app_info":
        return Promise.resolve(null);
      case "reorder_servers":
        return Promise.resolve(undefined);
      default:
        return Promise.resolve(null);
    }
  }) as typeof invoke);
});

describe("Servers OAuth", () => {
  it("未登录显示登录按钮并打开登录弹层", async () => {
    const oauthServer: ServerSummary = {
      ...alpha,
      id: "oauth",
      name: "OAuth",
      transport: "streamable_http",
      oauth: {
        configured: true,
        authorized: false,
        refreshable: false,
        scopes: ["read"],
        expires_at: null,
      },
      state: {
        state: "error",
        error: { kind: "oauth_required", message: "需要 OAuth 登录授权" },
      },
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([oauthServer]);
        case "get_app_info":
          return Promise.resolve(null);
        case "start_oauth_login":
          return Promise.resolve({
            flow_id: "flow-1",
            auth_url: "https://auth.example/authorize",
            redirect_uri: "http://127.0.0.1:33318/oauth/callback",
            flow_timeout_secs: 600,
            browser_opened: true,
            open_error: null,
            scopes: ["read"],
          });
        default:
          return Promise.resolve(null);
      }
    }) as typeof invoke);

    renderServers();
    expect(await screen.findByText("OAuth 需要登录")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "登录" }));
    expect(
      await screen.findByText(/OAuth 登录 · OAuth/),
    ).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("https://auth.example/authorize"),
    ).toBeInTheDocument();
  });

  it("Ready + step-up 显示缺少权限和补充授权", async () => {
    const oauthServer: ServerSummary = {
      ...alpha,
      transport: "streamable_http",
      oauth: {
        configured: true,
        authorized: true,
        refreshable: true,
        scopes: ["read"],
        expires_at: null,
        required_scope: "write",
        step_up_required: true,
      },
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([oauthServer]);
        case "get_app_info":
          return Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    }) as typeof invoke);

    renderServers();
    expect(await screen.findByText("OAuth 需要补充授权")).toBeInTheDocument();
    expect(screen.getByText(/缺少权限：/)).toHaveTextContent("write");
    expect(
      screen.getByRole("button", { name: "补充授权" }),
    ).toBeInTheDocument();
  });

  it("可清除客户端注册", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const oauthServer: ServerSummary = {
      ...alpha,
      transport: "streamable_http",
      oauth: {
        configured: true,
        authorized: true,
        refreshable: true,
        scopes: [],
        expires_at: null,
      },
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([oauthServer]);
        case "get_app_info":
          return Promise.resolve(null);
        case "clear_oauth_registration":
          return Promise.resolve({ cleared: true });
        default:
          return Promise.resolve(null);
      }
    }) as typeof invoke);

    renderServers();
    expect(await screen.findByText("OAuth 已登录")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "清除注册" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("clear_oauth_registration", {
        serverId: "alpha",
      }),
    );
    confirmSpy.mockRestore();
  });
});

describe("Servers 排序", () => {
  it("键盘下移手柄会提交完整 ID 顺序", async () => {
    renderServers();

    const handle = await screen.findByRole("button", {
      name: "调整「Alpha」顺序，使用上下方向键移动",
    });
    handle.focus();
    await userEvent.keyboard("{ArrowDown}");

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("reorder_servers", {
        ids: ["beta", "alpha"],
      }),
    );
  });
});
