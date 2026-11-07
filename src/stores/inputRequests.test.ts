import { beforeEach, describe, expect, it } from "vitest";
import { useInputRequests } from "./inputRequests";
import type { InputEvent } from "../types/input";

function opened(requestId: string): Extract<InputEvent, { type: "opened" }> {
  return {
    type: "opened",
    request_id: requestId,
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
        schema: { type: "object" },
      },
    ],
    expires_at: "2026-10-29T12:00:00Z",
  };
}

describe("inputRequests store", () => {
  beforeEach(() => {
    useInputRequests.getState().reset();
  });

  it("打开、切换与关闭", () => {
    useInputRequests.getState().open(opened("r1"));
    useInputRequests.getState().open(opened("r2"));
    expect(useInputRequests.getState().pending).toHaveLength(2);
    expect(useInputRequests.getState().activeId).toBe("r2");

    useInputRequests.getState().close("r2", "answered");
    expect(useInputRequests.getState().pending).toHaveLength(1);
    expect(useInputRequests.getState().activeId).toBe("r1");
  });

  it("最小化后恢复，并保留其他 pending", () => {
    useInputRequests.getState().open(opened("r1"));
    useInputRequests.getState().open(opened("r2"));
    useInputRequests.getState().minimize("r2");
    const state = useInputRequests.getState();
    expect(state.pending.find((item) => item.requestId === "r2")?.minimized).toBe(true);
    expect(state.activeId).toBe("r1");

    useInputRequests.getState().restore("r2");
    expect(useInputRequests.getState().activeId).toBe("r2");
    expect(
      useInputRequests.getState().pending.find((item) => item.requestId === "r2")
        ?.minimized,
    ).toBe(false);
  });
});
