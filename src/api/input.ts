import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { InputAnswer, InputEvent } from "../types/input";

/** 提交输入应答 */
export function respondInput(requestId: string, answers: InputAnswer[]): Promise<void> {
  return invoke<void>("respond_input", { requestId, answers });
}

/** 取消一次输入所属的调用 */
export function cancelInput(requestId: string): Promise<void> {
  return invoke<void>("cancel_input", { requestId });
}

/** 当前挂起的输入请求（HMR/刷新后恢复） */
export function listPendingInputs(): Promise<InputEvent[]> {
  return invoke<InputEvent[]>("list_pending_inputs");
}

/** 监听 `mcp://input` */
export function listenInputEvents(handler: (event: InputEvent) => void): Promise<UnlistenFn> {
  return listen<InputEvent>("mcp://input", (event) => handler(event.payload));
}
