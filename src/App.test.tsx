import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { queryClient } from "./lib/queryClient";

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
  queryClient.clear();
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "get_app_info":
        return Promise.resolve({
          version: "0.1.0",
          data_dir: "C:\\data",
          log_dir: "C:\\logs",
          supported_protocol_versions: ["2026-07-28", "2025-06-18"],
          portable: false,
          data_dir_source: "appdata",
          config_path: null,
          config_status: "not_found",
          config_error: null,
          config_changed: false,
          default_server_id: null,
          auto_connect: false,
        });
      case "list_servers":
        return Promise.resolve([]);
      default:
        return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
    }
  }) as typeof invoke);
});

describe("App", () => {
  it("渲染侧边栏导航与版本号", async () => {
    render(<App />);

    expect(screen.getByText("MCP Tester")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "服务器" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "接口" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "历史" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "设置" })).toBeInTheDocument();

    // 版本号经 get_app_info 展示
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
  });
});
