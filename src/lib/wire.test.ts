import { describe, expect, it } from "vitest";
import { pairWireEntries } from "./wire";
import type { WireEntry } from "../api/wire";

let seq = 0;

function entry(overrides: Partial<WireEntry>): WireEntry {
  seq += 1;
  return {
    seq,
    direction: "client_to_server",
    kind: "request",
    method: "tools/call",
    rpc_id: "1",
    summary: "",
    payload: {},
    truncated: false,
    bytes: 10,
    is_error: false,
    duration_ms: null,
    at: "2026-10-22T00:00:00Z",
    ...overrides,
  };
}

describe("pairWireEntries", () => {
  it("按 id 和方向配对请求/响应", () => {
    seq = 0;
    const request = entry({ seq: 1 });
    const response = entry({
      seq: 2,
      direction: "server_to_client",
      kind: "response",
      duration_ms: 12,
      is_error: false,
    });
    const items = pairWireEntries([request, response]);
    expect(items).toHaveLength(1);
    expect(items[0].request?.seq).toBe(1);
    expect(items[0].response?.seq).toBe(2);
  });

  it("多个请求同 id 时按顺序各取一条响应", () => {
    seq = 0;
    const first = entry({ seq: 1, rpc_id: "x" });
    const second = entry({ seq: 2, rpc_id: "x" });
    const firstResponse = entry({
      seq: 3,
      direction: "server_to_client",
      kind: "response",
      rpc_id: "x",
    });
    const secondResponse = entry({
      seq: 4,
      direction: "server_to_client",
      kind: "response",
      rpc_id: "x",
    });
    const items = pairWireEntries([first, second, firstResponse, secondResponse]);
    expect(items).toHaveLength(2);
    expect(items[0].request?.seq).toBe(1);
    expect(items[0].response?.seq).toBe(3);
    expect(items[1].request?.seq).toBe(2);
    expect(items[1].response?.seq).toBe(4);
  });

  it("响应先于请求时保持孤儿响应", () => {
    seq = 0;
    const response = entry({
      seq: 1,
      direction: "server_to_client",
      kind: "response",
      rpc_id: "1",
    });
    const request = entry({ seq: 2 });
    const items = pairWireEntries([response, request]);
    expect(items).toHaveLength(2);
    expect(items[0].response?.seq).toBe(1);
    expect(items[1].request?.seq).toBe(2);
    expect(items[1].response).toBeUndefined();
  });

  it("通知与系统事件各自成行，且保留 seq 顺序", () => {
    seq = 0;
    const notification = entry({
      seq: 1,
      kind: "notification",
      rpc_id: null,
      method: "notifications/message",
    });
    const request = entry({ seq: 2 });
    const system = entry({
      seq: 3,
      kind: "system",
      direction: "system",
      rpc_id: null,
      method: null,
    });
    const response = entry({
      seq: 4,
      direction: "server_to_client",
      kind: "response",
    });
    const items = pairWireEntries([notification, request, system, response]);
    expect(items.map((item) => item.key)).toEqual([
      "evt-1",
      "req-2",
      "evt-3",
    ]);
  });

  it("空 id 不参与配对", () => {
    seq = 0;
    const request = entry({ seq: 1, rpc_id: "" });
    const response = entry({
      seq: 2,
      direction: "server_to_client",
      kind: "response",
      rpc_id: "",
    });
    const items = pairWireEntries([request, response]);
    expect(items[0].response).toBeUndefined();
    expect(items[1].response?.seq).toBe(2);
  });

  it("不修改输入数组", () => {
    seq = 0;
    const request = entry({ seq: 1 });
    const response = entry({
      seq: 2,
      direction: "server_to_client",
      kind: "response",
    });
    const input = [request, response];
    pairWireEntries(input);
    expect(input).toEqual([request, response]);
  });
});
