/**
 * 调用记录的复制格式
 *
 * 结果区复制菜单用：请求 / 响应 / 成对内容都由 `CallRecord` 在前端重建，
 * 不新增后端字段。请求的 JSON-RPC id 使用本地记录 ID，与线上实际 id 可能不同；
 * HTTP 头与认证信息不在 `CallRecord` 中，因此不会被复制。
 */

import { CALL_METHOD_LABEL } from "./replay";
import { redactSensitive } from "./sensitive";
import type {
  CallError,
  CallFrames,
  CallRecord,
  CallStatus,
} from "../types/models";

const STATUS_LABEL: Record<CallStatus, string> = {
  ok: "成功",
  error: "失败",
  canceled: "已取消",
};

const ORIGIN_LABEL: Record<CallRecord["origin"], string> = {
  manual: "手工调用",
  test: "测试调用",
};

function frameSourceLabel(source: FrameSource): string {
  switch (source) {
    case "original":
      return "原始";
    case "reconstructed":
      return "重建";
    default:
      return "无";
  }
}

export type JsonRpcId = string | number;

export interface JsonRpcRequestEnvelope {
  jsonrpc: "2.0";
  id: JsonRpcId;
  method: string;
  params: unknown;
}

export interface JsonRpcErrorPayload {
  code: number | null;
  message: string;
  data?: unknown;
}

export interface JsonRpcResponseEnvelope {
  jsonrpc: "2.0";
  id: JsonRpcId;
  result?: unknown;
  error?: JsonRpcErrorPayload;
}

export type FrameSource = "original" | "reconstructed" | "none";

export interface FrameSources {
  request: Exclude<FrameSource, "none">;
  response: FrameSource;
}

export interface ExchangeMeta {
  record_id: string;
  server: string;
  server_id: string;
  transport: string;
  method: string;
  target: string;
  status: CallStatus;
  duration_ms: number;
  protocol_version: string;
  created_at: string;
  origin: string;
  truncated: boolean;
  size_bytes: number;
  frame_source: FrameSources;
}

export type ResponsePayload =
  | { kind: "result"; value: unknown }
  | { kind: "error"; error: CallError }
  | { kind: "none" };

/** JSON 美化输出；非法/空值时退化为 `null` */
export function stringifyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? "null";
  } catch {
    return String(value);
  }
}

function maybeRedact(value: unknown, redact: boolean): unknown {
  return redact ? redactSensitive(value).value : value;
}

/** 调用元信息（成对 Markdown / JSON 共用） */
export function buildExchangeMeta(
  record: CallRecord,
  frames?: CallFrames | null,
  fullResult?: unknown,
): ExchangeMeta {
  const hasResponse =
    fullResult !== undefined ||
    record.result !== null ||
    record.result !== undefined ||
    Boolean(record.error) ||
    Boolean(frames?.response);
  return {
    record_id: record.id,
    server: record.server_name,
    server_id: record.server_id,
    transport: record.transport,
    method: CALL_METHOD_LABEL[record.method],
    target: record.target,
    status: record.status,
    duration_ms: record.duration_ms,
    protocol_version: record.protocol_version,
    created_at: record.created_at,
    origin: record.origin,
    truncated: record.truncated,
    size_bytes: record.size_bytes,
    frame_source: {
      request: frames?.request ? "original" : "reconstructed",
      response:
        frames?.response && fullResult === undefined
          ? "original"
          : hasResponse
            ? "reconstructed"
            : "none",
    },
  };
}

/**
 * 重建 JSON-RPC 请求。
 *
 * `probe` 是应用层探测（现代世代 discover / 旧世代 ping），没有可重建的
 * 标准 JSON-RPC 请求，返回 null。
 */
export function buildRequestEnvelope(
  record: CallRecord,
  frames?: CallFrames | null,
): JsonRpcRequestEnvelope | null {
  if (record.method === "probe") return null;
  if (frames?.request && typeof frames.request === "object") {
    return frames.request as JsonRpcRequestEnvelope;
  }
  return {
    jsonrpc: "2.0",
    id: frames?.rpc_id ?? record.id,
    method: CALL_METHOD_LABEL[record.method],
    params: requestParams(record),
  };
}

/**
 * 复制请求时对记录参数做最小归一化：
 * - tools/prompts 的 `arguments` 为 null/缺省时，实际请求不会带该字段；
 * - 其它字段保持原样。
 */
function requestParams(record: CallRecord): unknown {
  const params = record.params;
  if (!params || typeof params !== "object" || Array.isArray(params)) {
    return params ?? {};
  }
  if (record.method !== "tools_call" && record.method !== "prompts_get") {
    return params;
  }
  const candidate = params as Record<string, unknown>;
  if (candidate.arguments !== null && candidate.arguments !== undefined) {
    return params;
  }
  const rest = { ...candidate };
  delete rest.arguments;
  return rest;
}

/**
 * 取响应的“内容”：
 * - 有 result 时优先 result（包括 MCP 工具 `isError=true` 的情况）；
 * - 没有 result 时取应用层 error；
 * - 取消/无结果时返回 none。
 *
 * `fullResult` 用于截断记录：调用方先读取 `results/<id>.json`，再传进来。
 */
export function responsePayload(
  record: CallRecord,
  fullResult?: unknown,
): ResponsePayload {
  if (fullResult !== undefined) {
    return { kind: "result", value: fullResult };
  }
  if (record.result !== null && record.result !== undefined) {
    return { kind: "result", value: record.result };
  }
  if (record.error) {
    return { kind: "error", error: record.error };
  }
  return { kind: "none" };
}

/**
 * 重建 JSON-RPC 响应。
 *
 * 只有“result 存在”或“error 是真正的 RPC 错误”时才返回信封；
 * 应用层错误（超时、传输关闭、OAuth 等）没有 JSON-RPC 响应，返回 null。
 * `probe` 的结果是应用层 `ProbeResult`，同样不伪装成 JSON-RPC 响应。
 */
export function buildResponseEnvelope(
  record: CallRecord,
  fullResult?: unknown,
  frames?: CallFrames | null,
): JsonRpcResponseEnvelope | null {
  if (record.method === "probe") return null;
  // 读取了完整结果时优先用完整 body 重建，避免原始帧中的截断结果
  if (frames?.response && fullResult === undefined) {
    return frames.response as JsonRpcResponseEnvelope;
  }
  const id = frames?.rpc_id ?? record.id;
  const payload = responsePayload(record, fullResult);
  if (payload.kind === "result") {
    return { jsonrpc: "2.0", id, result: payload.value };
  }
  if (payload.kind === "error") {
    const error = rpcErrorOf(payload.error);
    if (error) {
      return { jsonrpc: "2.0", id, error };
    }
  }
  return null;
}

/** 请求 JSON-RPC 信封文本；探测记录返回 null */
export function buildRequestText(
  record: CallRecord,
  frames?: CallFrames | null,
  redact = false,
): string | null {
  const envelope = buildRequestEnvelope(record, frames);
  return envelope ? stringifyJson(maybeRedact(envelope, redact)) : null;
}

/** 只有请求参数（`params`），便于贴到别处 */
export function buildRequestParamsText(
  record: CallRecord,
  redact = false,
): string {
  return stringifyJson(maybeRedact(requestParams(record), redact));
}

/** 响应 body（result 或应用层 error）；无响应返回 null */
export function buildResponseBodyText(
  record: CallRecord,
  fullResult?: unknown,
  redact = false,
): string | null {
  const payload = responsePayload(record, fullResult);
  if (payload.kind === "none") return null;
  return stringifyJson(
    maybeRedact(
      payload.kind === "result" ? payload.value : payload.error,
      redact,
    ),
  );
}

/** 完整 JSON-RPC 响应信封；应用层错误/探测记录返回 null */
export function buildResponseEnvelopeText(
  record: CallRecord,
  fullResult?: unknown,
  frames?: CallFrames | null,
  redact = false,
): string | null {
  const envelope = buildResponseEnvelope(record, fullResult, frames);
  return envelope ? stringifyJson(maybeRedact(envelope, redact)) : null;
}

/** 成对 Markdown：适合贴 issue / 聊天 / 文档 */
export function buildPairMarkdown(
  record: CallRecord,
  fullResult?: unknown,
  frames?: CallFrames | null,
  redact = false,
): string {
  const payload = responsePayload(record, fullResult);
  const meta = buildExchangeMeta(record, frames, fullResult);
  const lines: string[] = [
    "### MCP 调用记录",
    "",
    `- 服务器：${record.server_name}（${record.transport}）`,
    `- 方法：${CALL_METHOD_LABEL[record.method]} · ${record.target}`,
    `- 状态：${STATUS_LABEL[record.status]} · ${record.duration_ms}ms`,
    `- 协议：${record.protocol_version || "未知协议"}`,
    `- 时间：${record.created_at}`,
    `- 来源：${ORIGIN_LABEL[record.origin]}`,
    `- 记录：${record.id}`,
    `- 帧来源：请求 ${frameSourceLabel(meta.frame_source.request)}；响应 ${frameSourceLabel(meta.frame_source.response)}`,
  ];

  if (record.error?.kind === "validation") {
    lines.push("- 备注：请求未实际发送（参数校验失败）");
  }
  if (frames?.truncated) {
    lines.push("- 备注：部分原始帧未保存，已按记录重建");
  }
  if (record.truncated) {
    lines.push(
      "",
      fullResult === undefined
        ? "> 结果已截断；本次复制未包含完整响应。"
        : "> 结果已截断；已读取落盘的完整内容。",
    );
  }

  lines.push("", "**请求**", "");
  const request = buildRequestEnvelope(record, frames);
  lines.push(
    request
      ? fencedJson(maybeRedact(request, redact))
      : "_探测记录没有标准 JSON-RPC 请求。_",
  );

  lines.push("", "**响应**", "");
  if (payload.kind === "result") {
    if (record.method === "probe") {
      lines.push(
        "_探测结果（应用层）_",
        "",
        fencedJson(maybeRedact(payload.value, redact)),
      );
    } else {
      const envelope = buildResponseEnvelope(record, fullResult, frames);
      lines.push(
        envelope
          ? fencedJson(maybeRedact(envelope, redact))
          : fencedJson(maybeRedact(payload.value, redact)),
      );
    }
    if (record.error) {
      const message = record.error.message ?? record.error.kind;
      const shown = redact ? redactSensitive(message).value : message;
      lines.push("", `_应用层诊断：${String(shown)}_`);
    }
  } else if (payload.kind === "error") {
    const rpcError = rpcErrorOf(payload.error);
    if (rpcError && record.method !== "probe") {
      lines.push(
        fencedJson(
          maybeRedact(
            {
              jsonrpc: "2.0",
              id: frames?.rpc_id ?? record.id,
              error: rpcError,
            },
            redact,
          ),
        ),
      );
    } else {
      lines.push(
        "_应用层错误（没有 JSON-RPC 响应）_",
        "",
        fencedJson(maybeRedact(payload.error, redact)),
      );
    }
  } else if (record.status === "canceled") {
    lines.push("_调用已取消，无响应。_");
  } else if (record.truncated) {
    lines.push("_结果已截断，完整响应未读取。_");
  } else {
    lines.push("_无响应。_");
  }

  return lines.join("\n");
}

/**
 * 成对 JSON：适合脚本 / 后续导入。
 *
 * `response` 只在有标准 JSON-RPC 响应时存在；`result` 是响应 body（含探测结果）；
 * `app_error` 是调用记录里的应用层错误（含 `isError=true` 的派生诊断）。
 */
export function buildPairJson(
  record: CallRecord,
  fullResult?: unknown,
  frames?: CallFrames | null,
  redact = false,
): string {
  const payload = responsePayload(record, fullResult);
  const response = buildResponseEnvelope(record, fullResult, frames);
  const appError =
    response !== null && payload.kind === "error"
      ? null
      : (record.error ?? null);
  return stringifyJson(
    maybeRedact(
      {
        meta: buildExchangeMeta(record, frames, fullResult),
        request: buildRequestEnvelope(record, frames),
        response,
        result: payload.kind === "result" ? payload.value : null,
        app_error: appError,
      },
      redact,
    ),
  );
}

function rpcErrorOf(error: CallError): JsonRpcErrorPayload | null {
  if (error.kind !== "rpc_error") return null;
  const code = typeof error.code === "number" ? error.code : null;
  const message =
    typeof error.message === "string" && error.message
      ? error.message
      : "未知错误";
  return {
    code,
    message,
    ...(error.data !== undefined ? { data: error.data } : {}),
  };
}

/** Markdown 代码围栏：内容里出现反引号时自动加长，避免围栏被提前闭合 */
function fencedJson(value: unknown): string {
  const text = stringifyJson(value);
  const runs = text.match(/`+/g) ?? [];
  const longest = runs.reduce((max, run) => Math.max(max, run.length), 0);
  const fence = "`".repeat(Math.max(3, longest + 1));
  return `${fence}json\n${text}\n${fence}`;
}
