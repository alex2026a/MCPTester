import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EventPanel } from "./EventPanel";
import { getWireLog } from "../api/wire";
import type { WireEntry } from "../api/wire";

vi.mock("../api/wire", () => ({
  getWireLog: vi.fn(),
  listenWire: vi.fn().mockResolvedValue(() => {}),
  clearWireLog: vi.fn(),
  exportWireLog: vi.fn(),
}));

const getWireLogMock = vi.mocked(getWireLog);

function entry(overrides: Partial<WireEntry>): WireEntry {
  return {
    seq: 1,
    direction: "client_to_server",
    kind: "request",
    method: "tools/call",
    rpc_id: "1",
    summary: "request tools/call",
    payload: { jsonrpc: "2.0", id: 1, method: "tools/call" },
    truncated: false,
    bytes: 10,
    is_error: false,
    duration_ms: null,
    at: "2026-10-22T00:00:00Z",
    ...overrides,
  };
}

function renderPanel(onOpenChange?: (open: boolean) => void) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <EventPanel serverId="s1" onOpenChange={onOpenChange} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getWireLogMock.mockReset();
});

describe("EventPanel", () => {
  it("按 id 配对请求和响应并展示生效方法", async () => {
    getWireLogMock.mockResolvedValue([
      entry({ seq: 1 }),
      entry({
        seq: 2,
        direction: "server_to_client",
        kind: "response",
        method: null,
        payload: { jsonrpc: "2.0", id: 1, result: { ok: true } },
        duration_ms: 8,
      }),
    ]);
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: /事件面板/ }));
    await waitFor(() => expect(getWireLogMock).toHaveBeenCalledWith("s1"));
    expect((await screen.findAllByText("tools/call")).length).toBeGreaterThan(0);
    expect(screen.getByText(/缓冲/).textContent).toContain("共 1 条");
  });

  it("展开/收起时通知 onOpenChange", async () => {
    getWireLogMock.mockResolvedValue([]);
    const onOpenChange = vi.fn();
    renderPanel(onOpenChange);
    const toggle = screen.getByRole("button", { name: /事件面板/ });

    await userEvent.click(toggle);
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
    await userEvent.click(toggle);
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
  });
});
