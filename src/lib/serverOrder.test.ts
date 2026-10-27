import { describe, expect, it } from "vitest";
import {
  groupServers,
  moveItem,
  pickDefaultServerId,
  serverOptionLabel,
  serverStatusMeta,
} from "./serverOrder";
import type { ConnectionState, ServerSummary } from "../types/models";

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

function server(
  id: string,
  state: ConnectionState,
  enabled = true,
): ServerSummary {
  return {
    id,
    name: `Server ${id}`,
    transport: "stdio",
    enabled,
    tags: [],
    last_connected_at: null,
    state,
  };
}

describe("serverOrder", () => {
  it("按已连接、连接中、未连接、异常、已停用分组，组内保持输入顺序", () => {
    const groups = groupServers([
      server("d1", { state: "disconnected" }),
      server("r1", ready),
      server("e1", { state: "error", error: { kind: "internal" } }),
      server("c1", { state: "connecting" }),
      server("r2", ready),
      server("x1", { state: "disconnected" }, false),
    ]);

    expect(groups.map((group) => group.key)).toEqual([
      "connected",
      "connecting",
      "disconnected",
      "error",
      "disabled",
    ]);
    expect(groups[0].label).toBe("已连接");
    expect(groups[0].servers.map((item) => item.id)).toEqual(["r1", "r2"]);
    expect(groups[1].servers.map((item) => item.id)).toEqual(["c1"]);
    expect(groups[2].servers.map((item) => item.id)).toEqual(["d1"]);
    expect(groups[3].servers.map((item) => item.id)).toEqual(["e1"]);
    expect(groups[4].servers.map((item) => item.id)).toEqual(["x1"]);
  });

  it("选项文本把状态字形放在名称前面", () => {
    expect(serverOptionLabel(server("r", ready), true)).toBe("● Server r");
    expect(serverOptionLabel(server("r", ready), false)).toBe(
      "● Server r（已连接）",
    );
    expect(
      serverOptionLabel(
        server("d", { state: "degraded", reason: "连续失败" }),
        true,
      ),
    ).toBe("◐ Server d（降级）");
    expect(
      serverOptionLabel(server("x", { state: "disconnected" }, false), true),
    ).toBe("× Server x（已停用）");
    expect(
      serverStatusMeta(
        server("e", { state: "lost", reason: "退出", can_reconnect: true }),
      ).label,
    ).toBe("掉线");
  });

  it("默认服务器优先显式配置，其次第一个可用服务器", () => {
    const servers = [
      server("d", { state: "disconnected" }),
      server("r", ready),
    ];
    expect(pickDefaultServerId(servers, "d")).toBe("d");
    expect(pickDefaultServerId(servers, "missing")).toBe("r");
    expect(pickDefaultServerId(servers)).toBe("r");
    expect(pickDefaultServerId([server("d", { state: "disconnected" })])).toBe(
      "d",
    );
    expect(pickDefaultServerId([])).toBe("");
  });

  it("moveItem 处理上下移动与越界", () => {
    expect(moveItem(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
    expect(moveItem(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(moveItem(["a", "b", "c"], 0, 1)).toEqual(["b", "a", "c"]);
    const items = ["a", "b"];
    expect(moveItem(items, -1, 1)).toBe(items);
    expect(moveItem(items, 1, 2)).toBe(items);
  });
});
