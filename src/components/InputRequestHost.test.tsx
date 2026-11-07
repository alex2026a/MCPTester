import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InputRequestHost } from "./InputRequestHost";
import { useInputRequests } from "../stores/inputRequests";
import type { InputEvent } from "../types/input";

const listPendingInputs = vi.fn();
const listenInputEvents = vi.fn();
const respondInput = vi.fn();
const cancelInput = vi.fn();
let handler: ((event: InputEvent) => void) | null = null;

vi.mock("../api/input", () => ({
  listPendingInputs: () => listPendingInputs(),
  listenInputEvents: (callback: (event: InputEvent) => void) => {
    handler = callback;
    return listenInputEvents(callback);
  },
  respondInput: (requestId: string, answers: unknown[]) =>
    respondInput(requestId, answers),
  cancelInput: (requestId: string) => cancelInput(requestId),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
}));

function opened(): InputEvent {
  return {
    type: "opened",
    request_id: "r1",
    context: {
      server_id: "s1",
      server_name: "Demo",
      call_id: "c1",
      method: "tools_call",
      target: "echo",
      round: 1,
    },
    requests: [
      {
        kind: "elicitation",
        key: "name",
        mode: "form",
        message: "请输入名称",
        schema: {
          type: "object",
          properties: { name: { type: "string" } },
          required: ["name"],
        },
      },
    ],
    expires_at: new Date(Date.now() + 300_000).toISOString(),
  };
}

describe("InputRequestHost", () => {
  beforeEach(() => {
    useInputRequests.getState().reset();
    handler = null;
    listPendingInputs.mockReset().mockResolvedValue([]);
    listenInputEvents.mockReset().mockResolvedValue(() => {});
    respondInput.mockReset().mockResolvedValue(undefined);
    cancelInput.mockReset().mockResolvedValue(undefined);
  });

  it("监听事件、提交后关闭", async () => {
    render(<InputRequestHost />);
    await waitFor(() => expect(listenInputEvents).toHaveBeenCalledTimes(1));

    act(() => {
      handler?.(opened());
    });
    expect(screen.getByText("服务器请求输入")).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("name"), "Ada");
    await userEvent.click(screen.getByRole("button", { name: "提交" }));
    await waitFor(() =>
      expect(respondInput).toHaveBeenCalledWith("r1", [
        { key: "name", action: "accept", content: { name: "Ada" } },
      ]),
    );
    await waitFor(() =>
      expect(screen.queryByText("服务器请求输入")).not.toBeInTheDocument(),
    );
  });

  it("挂载时恢复待处理请求", async () => {
    listPendingInputs.mockResolvedValue([opened()]);
    render(<InputRequestHost />);
    expect(await screen.findByText("服务器请求输入")).toBeInTheDocument();
  });
});
