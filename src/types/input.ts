/** 交互输入事件与应答（v0.6.0） */

import type { CallMethod } from "./models";

export type InputAction = "accept" | "decline";

export interface InputContext {
  server_id: string;
  server_name: string;
  call_id: string | null;
  method: CallMethod | null;
  target: string | null;
  /** MRTR 轮次（1-based）；旧世代固定 1 */
  round: number;
}

export interface InputRequestView {
  kind: "elicitation";
  key: string;
  mode: "form" | "url";
  message: string;
  schema?: unknown;
  url?: string;
  elicitation_id?: string;
}

export type InputEvent =
  | {
      type: "opened";
      request_id: string;
      context: InputContext;
      requests: InputRequestView[];
      expires_at: string;
    }
  | {
      type: "closed";
      request_id: string;
      reason: "answered" | "timeout" | "canceled" | "disconnected" | "call_finished" | string;
    };

export interface InputAnswer {
  key: string;
  action: InputAction;
  content?: unknown;
}
