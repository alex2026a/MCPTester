import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { onlineManager, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { History } from "./History";
import { createQueryClient } from "../lib/queryClient";
import type {
  CallRecord,
  CallRecordSummary,
  ServerSummary,
} from "../types/models";

const invokeMock = vi.mocked(invoke);

const server: ServerSummary = {
  id: "srv-1",
  name: "Mock Server",
  transport: "stdio",
  enabled: true,
  tags: [],
  last_connected_at: null,
  state: { state: "disconnected" },
};

const record: CallRecordSummary = {
  id: "rec-1",
  server_id: "srv-1",
  server_name: "Mock Server",
  method: "tools_call",
  target: "echo",
  status: "ok",
  duration_ms: 12,
  size_bytes: 34,
  truncated: false,
  origin: "manual",
  created_at: "2026-10-27T09:00:00Z",
};

const detail: CallRecord = {
  id: "rec-1",
  server_id: "srv-1",
  server_name: "Mock Server",
  method: "tools_call",
  target: "echo",
  params: { name: "echo", arguments: { text: "hi" } },
  result: { content: [{ type: "text", text: "hi" }] },
  error: null,
  status: "ok",
  duration_ms: 12,
  protocol_version: "2026-07-28",
  transport: "stdio",
  truncated: false,
  size_bytes: 34,
  origin: "manual",
  frames: null,
  created_at: "2026-10-27T09:00:00Z",
};

function renderHistory(entry: string) {
  const client = createQueryClient({ queries: { retry: false } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[entry]}>
        <History />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "list_servers":
        return Promise.resolve([server]);
      case "list_call_records":
        return Promise.resolve({ items: [], total: 0, offset: 0, limit: 200 });
      default:
        return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
    }
  }) as typeof invoke);
});

afterEach(() => {
  onlineManager.setOnline(true);
});

describe("History target 深链", () => {
  it("从 URL 读取 target 并显示可清除 chip", async () => {
    renderHistory(
      "/history?server_id=srv-1&method=tools_call&target=echo&origin=all",
    );

    expect(await screen.findByText("echo")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith(
      "list_call_records",
      expect.objectContaining({
        filter: expect.objectContaining({
          server_id: "srv-1",
          method: "tools_call",
          target: "echo",
          origin: null,
        }),
      }),
    );

    await userEvent.click(screen.getByRole("button", { name: "清除接口筛选" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenLastCalledWith(
        "list_call_records",
        expect.objectContaining({
          filter: expect.objectContaining({ target: null }),
        }),
      ),
    );
  });
});

describe("历史详情", () => {
  it("系统离线时仍能读取本地详情", async () => {
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "list_call_records":
          return Promise.resolve({
            items: [record],
            total: 1,
            offset: 0,
            limit: 200,
          });
        case "get_call_record":
          return Promise.resolve(detail);
        default:
          return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
      }
    }) as typeof invoke);
    renderHistory("/history");

    const item = await screen.findByRole("button", { name: /echo/ });
    onlineManager.setOnline(false);
    await userEvent.click(item);

    expect(
      await screen.findByRole("button", { name: "载入调用台重放" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("详情加载失败")).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("get_call_record", {
      recordId: "rec-1",
    });
  });

  it("详情读取失败时展示错误信息并可重试", async () => {
    let detailAttempts = 0;
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "list_call_records":
          return Promise.resolve({
            items: [record],
            total: 1,
            offset: 0,
            limit: 200,
          });
        case "get_call_record":
          detailAttempts += 1;
          return detailAttempts === 1
            ? Promise.reject(new Error("数据库读锁异常"))
            : Promise.resolve(detail);
        default:
          return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
      }
    }) as typeof invoke);
    renderHistory("/history");

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));

    expect(await screen.findByText("详情加载失败")).toBeInTheDocument();
    expect(screen.getByText("数据库读锁异常")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "重试" }));

    expect(
      await screen.findByRole("button", { name: "载入调用台重放" }),
    ).toBeInTheDocument();
    expect(detailAttempts).toBe(2);
  });
});
