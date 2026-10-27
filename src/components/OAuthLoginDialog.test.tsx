import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { OAuthLoginDialog } from "./OAuthLoginDialog";
import type { OAuthPushPayload } from "../api/oauth";

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);
let handler: ((event: { payload: OAuthPushPayload }) => void) | null = null;

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  handler = null;
  listenMock.mockImplementation(((_event: string, cb: typeof handler) => {
    handler = cb;
    return Promise.resolve(() => {});
  }) as typeof listen);
  invokeMock.mockImplementation(((cmd: string) => {
    if (cmd === "start_oauth_login") {
      return Promise.resolve({
        flow_id: "flow-1",
        auth_url: "https://auth.example/authorize",
        redirect_uri: "http://127.0.0.1:33318/oauth/callback",
        flow_timeout_secs: 600,
        browser_opened: true,
        open_error: null,
        callback_warning: "localhost 的 IPv6 回调不可用",
        scopes: ["read"],
      });
    }
    return Promise.resolve(undefined);
  }) as typeof invoke);
});

describe("OAuthLoginDialog", () => {
  it("只处理当前 flow 的 authorized 事件", async () => {
    const onAuthorized = vi.fn();
    const onClose = vi.fn();
    render(
      <OAuthLoginDialog
        serverId="srv-1"
        serverName="Demo"
        onClose={onClose}
        onAuthorized={onAuthorized}
      />,
    );

    expect(
      await screen.findByDisplayValue("https://auth.example/authorize"),
    ).toBeInTheDocument();
    expect(screen.getByText(/请求权限：/)).toHaveTextContent("read");
    expect(
      screen.getByText(/localhost 的 IPv6 回调不可用/),
    ).toBeInTheDocument();

    act(() => {
      handler?.({
        payload: {
          server_id: "other",
          flow_id: "flow-other",
          state: "authorized",
        },
      });
    });
    expect(onAuthorized).not.toHaveBeenCalled();

    act(() => {
      handler?.({
        payload: { server_id: "srv-1", flow_id: "flow-1", state: "authorized" },
      });
    });
    await waitFor(() => expect(onAuthorized).toHaveBeenCalledTimes(1));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("error 事件展示错误并可重试", async () => {
    render(
      <OAuthLoginDialog
        serverId="srv-1"
        serverName="Demo"
        onClose={() => {}}
        onAuthorized={() => {}}
      />,
    );
    await screen.findByDisplayValue("https://auth.example/authorize");

    act(() => {
      handler?.({
        payload: {
          server_id: "srv-1",
          flow_id: "flow-1",
          state: "error",
          message: "token 交换失败",
        },
      });
    });
    expect(await screen.findByText("token 交换失败")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();
  });
});
