import { invoke } from "@tauri-apps/api/core";
import type { CallRecord } from "../types/models";

/** 调用工具（call_id 预置：便于取消） */
export function callTool(
  id: string,
  callId: string,
  name: string,
  args: unknown,
): Promise<CallRecord> {
  return invoke<CallRecord>("call_tool", { id, callId, name, arguments: args });
}

/** 读取资源 */
export function readResource(id: string, uri: string): Promise<CallRecord> {
  return invoke<CallRecord>("read_resource", { id, uri });
}

/** 获取提示词 */
export function getPrompt(id: string, name: string, args: unknown): Promise<CallRecord> {
  return invoke<CallRecord>("get_prompt", { id, name, arguments: args });
}

/** 取消在途调用 */
export function cancelCall(id: string, callId: string): Promise<boolean> {
  return invoke<boolean>("cancel_call", { id, callId });
}
