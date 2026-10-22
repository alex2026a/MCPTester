/**
 * JSON-RPC 报文配对（v0.2.2）
 *
 * 把 `WireEntry` 按 `(direction, rpc_id)` 配对成请求/响应交互，供 EventPanel 与
 * 测试报告用例详情共用。规则：
 * - 请求只匹配反方向、同 id、seq 晚于请求且尚未消费的响应；
 * - 多个请求同 id 时按顺序各取一条响应；
 * - 响应先于请求、孤儿响应保持独立行，不反向绑定；
 * - 输出按原始 seq 升序，保证时间线语义；
 * - 不修改输入数组。
 */

import type { WireEntry } from "../api/wire";

export interface WireInteraction {
  key: string;
  request?: WireEntry;
  response?: WireEntry;
  entry?: WireEntry;
}

function hasId(entry: WireEntry): boolean {
  return entry.rpc_id !== null && entry.rpc_id !== "";
}

export function pairWireEntries(wire: WireEntry[]): WireInteraction[] {
  const responses = new Map<string, WireEntry[]>();
  for (const entry of wire) {
    if (entry.kind !== "response" || !hasId(entry)) continue;
    const key = `${entry.direction}:${entry.rpc_id}`;
    const list = responses.get(key);
    if (list) {
      list.push(entry);
    } else {
      responses.set(key, [entry]);
    }
  }

  const consumed = new Set<number>();
  const paired = new Map<number, WireEntry>();
  for (const entry of wire) {
    if (entry.kind !== "request" || !hasId(entry)) continue;
    const opposite =
      entry.direction === "client_to_server"
        ? "server_to_client"
        : entry.direction === "server_to_client"
          ? "client_to_server"
          : null;
    if (!opposite) continue;
    const candidates = responses.get(`${opposite}:${entry.rpc_id}`) ?? [];
    const response = candidates.find(
      (item) => item.seq > entry.seq && !consumed.has(item.seq),
    );
    if (!response) continue;
    consumed.add(response.seq);
    paired.set(entry.seq, response);
  }

  const items: WireInteraction[] = [];
  for (const entry of wire) {
    if (entry.kind === "request") {
      items.push({
        key: `req-${entry.seq}`,
        request: entry,
        response: paired.get(entry.seq),
      });
    } else if (entry.kind === "response") {
      if (consumed.has(entry.seq)) continue;
      items.push({ key: `res-${entry.seq}`, response: entry });
    } else {
      items.push({ key: `evt-${entry.seq}`, entry });
    }
  }
  return items;
}
