import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 报文方向（Rust `WireDirection`） */
export type WireDirection = "client_to_server" | "server_to_client" | "system";

/** 报文类型（Rust `WireKind`） */
export type WireKind = "request" | "response" | "notification" | "system";

/** 线上报文条目（Rust `mcp::wire::WireEntry`） */
export interface WireEntry {
  seq: number;
  direction: WireDirection;
  kind: WireKind;
  method: string | null;
  rpc_id: string | null;
  summary: string;
  payload: unknown;
  truncated: boolean;
  bytes: number;
  is_error: boolean;
  duration_ms: number | null;
  at: string;
}

/** 报文增量推送载荷（`mcp://wire`） */
export interface WirePushPayload {
  server_id: string;
  entries: WireEntry[];
  /** 增量缺口（缓冲已滚出）→ 前端应重新全量拉取 */
  dropped: boolean;
}

/** 读取指定服务器的线上报文日志（未连接时为空列表） */
export function getWireLog(serverId: string): Promise<WireEntry[]> {
  return invoke<WireEntry[]>("get_wire_log", { serverId });
}

/** 清空指定服务器的报文缓冲 */
export function clearWireLog(serverId: string): Promise<boolean> {
  return invoke<boolean>("clear_wire_log", { serverId });
}

/** 导出报文（返回写入路径） */
export function exportWireLog(serverId: string, path?: string): Promise<string> {
  return invoke<string>("export_wire_log", { serverId, path: path ?? null });
}

/** 订阅报文增量推送（v0.1.3） */
export function listenWire(
  handler: (payload: WirePushPayload) => void,
): Promise<UnlistenFn> {
  return listen<WirePushPayload>("mcp://wire", (event) => handler(event.payload));
}
