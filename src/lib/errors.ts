/**
 * 统一错误信息模型（v0.1.1）
 *
 * - 结构化来源：`connect_server` / `reconnect_server` 失败返回
 *   `{ kind, message, raw }`；连接状态中的 `McpError`（kind + 字段）。
 * - 展示模型：人话标题 + 消息 + 修复建议 + 原始技术细节（可复制）。
 * - 其余命令（CRUD / 导入导出等）仍返回字符串，统一归一到"操作失败"。
 */

/** Rust 侧结构化错误载荷（McpErrorInfo / McpError serde 形态） */
export interface ApiErrorInfo {
  kind: string;
  message?: string;
  raw?: string | null;
  // McpError 变体字段
  status?: number;
  detail?: string;
  supported?: string[];
  timeout_secs?: number;
}

/** UI 展示模型 */
export interface ErrorInfo {
  /** 结构化错误类别（普通字符串错误为 null） */
  kind: string | null;
  /** 人话标题（如"认证失败"） */
  title: string;
  /** 可读消息 */
  message: string;
  /** 修复建议 */
  hint: string | null;
  /** 原始技术细节（SDK / 服务器原文） */
  raw: string | null;
}

/** 错误类别 → 人话标题与修复建议（UI 唯一出处） */
const KIND_META: Record<string, { title: string; hint: string }> = {
  spawn_failed: {
    title: "启动进程失败",
    hint: "确认命令在 PATH 中；可在编辑界面把「命令解析方式」改为 cmd_shell，或填写可执行文件完整路径。",
  },
  transport_failed: {
    title: "连接失败（网络/传输）",
    hint: "检查 URL 是否可达、服务是否已启动；本地自签证书可开启「允许不安全 TLS」。",
  },
  handshake_failed: {
    title: "握手失败",
    hint: "确认对端是 MCP 服务；若为旧版服务器，可在高级项固定协议版本（如 2025-11-25）后重试。",
  },
  unsupported_protocol_version: {
    title: "协议版本不受支持",
    hint: "在编辑界面高级项固定协议版本（如 2025-11-25）后重试。",
  },
  unauthorized: {
    title: "认证失败",
    hint: "检查 Authorization 请求头：填完整「Bearer <token>」或只填 token 均可（客户端会自动补 Bearer 前缀）；确认 token 未过期或未被轮换。",
  },
  unsupported_client_capability: {
    title: "服务器请求了不支持的能力",
    hint: "该场景 v0.1 不支持（如 elicitation），属已知边界；可改用旧世代服务器复现。",
  },
  connect_timeout: {
    title: "连接超时",
    hint: "确认服务运行状态与网络；可在设置页调大连接超时后重试。",
  },
  config: {
    title: "配置错误",
    hint: "检查服务器配置（URL / 请求头 / 命令 / 工作目录）后重试。",
  },
  credential_mask: {
    title: "凭据需要重新填写",
    hint: "凭据库中的敏感值是掩码（由旧版本编辑保存遗留，无法用于连接）。请编辑服务器：清空对应敏感值输入框，重新粘贴真实值后保存重试。",
  },
  internal: {
    title: "内部错误",
    hint: "可复制原始细节用于排障，或查看事件面板「报文」页签了解协议交互过程。",
  },
};

/** 错误类别 → 人话标题（无映射时回退「错误」） */
export function errorTitle(kind: string | null | undefined): string {
  if (!kind) return "错误";
  return KIND_META[kind]?.title ?? `错误（${kind}）`;
}

function isStructuredError(
  error: unknown,
): error is ApiErrorInfo & { kind: string } {
  return (
    !!error &&
    typeof error === "object" &&
    typeof (error as { kind?: unknown }).kind === "string"
  );
}

/** 由结构化 `McpError` 字段拼装可读消息（连接状态来源无中文 Display） */
function messageFromMcpError(error: ApiErrorInfo): string {
  switch (error.kind) {
    case "unauthorized":
      return error.detail
        ? `HTTP ${error.status ?? 401} · ${error.detail}`
        : `HTTP ${error.status ?? 401}`;
    case "unsupported_protocol_version":
      return error.supported && error.supported.length > 0
        ? `服务器支持：${error.supported.join("、")}`
        : "服务器未提供可用协议版本";
    case "connect_timeout":
      return error.timeout_secs ? `超过 ${error.timeout_secs}s 未完成连接` : "连接超时";
    default:
      return error.message ?? error.detail ?? error.kind;
  }
}

/** 任意错误 → 统一展示模型 */
export function toErrorInfo(error: unknown): ErrorInfo {
  if (isStructuredError(error)) {
    const meta = KIND_META[error.kind];
    const message =
      typeof error.message === "string" && error.message.length > 0
        ? error.message
        : messageFromMcpError(error);
    const raw =
      error.raw ??
      (error.detail && error.kind === "unauthorized" ? null : (error.detail ?? null));
    return {
      kind: error.kind,
      title: meta?.title ?? "操作失败",
      message,
      hint: meta?.hint ?? null,
      raw,
    };
  }

  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : JSON.stringify(error ?? "未知错误");
  return {
    kind: null,
    title: "操作失败",
    message: message || "未知错误",
    hint: null,
    raw: null,
  };
}
