/**
 * 调用记录复制菜单
 *
 * 一个主按钮 + 菜单，避免在结果区堆三个平级按钮：
 * - 主按钮默认复制响应 body（最高频）；
 * - 菜单提供请求、成对 Markdown/JSON、请求参数、完整 JSON-RPC 响应；
 * - 截断结果先读取 `results/<id>.json`，避免复制半截 JSON。
 */

import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import { readFullResult } from "../api/history";
import { getServer } from "../api/servers";
import { writeClipboardText } from "../lib/clipboard";
import { buildHttpSkeleton } from "../lib/httpSkeleton";
import { buildTextDigest } from "../lib/textDigest";
import {
  buildPairJson,
  buildPairMarkdown,
  buildRequestParamsText,
  buildRequestText,
  buildResponseBodyText,
  buildResponseEnvelopeText,
} from "../lib/exchange";
import { confirmLargeResultRead } from "../lib/largeResult";
import { findSensitiveFields } from "../lib/sensitive";
import { toErrorInfo } from "../lib/errors";
import { formatBytes } from "./json/format";
import { toast } from "../stores/toast";
import type { CallRecord } from "../types/models";

type CopyAction =
  | "request"
  | "response"
  | "pair-markdown"
  | "pair-json"
  | "params"
  | "response-envelope"
  | "http-skeleton"
  | "text-digest";

interface MenuEntry {
  action: CopyAction;
  label: string;
  hint: string;
  disabled: boolean;
  separator?: boolean;
}

interface Placement {
  left: number;
  width: number;
  maxHeight: number;
  top?: number;
  bottom?: number;
}

const MENU_WIDTH = 264;

const ACTION_LABEL: Record<CopyAction, string> = {
  request: "请求",
  response: "响应",
  "pair-markdown": "请求和响应（Markdown）",
  "pair-json": "请求和响应（JSON）",
  params: "请求参数",
  "response-envelope": "响应（JSON-RPC）",
  "http-skeleton": "HTTP 请求骨架",
  "text-digest": "文本内容",
};

export function CopyExchangeButton({
  record,
  primaryAction = "response",
}: {
  record: CallRecord;
  primaryAction?: "request" | "response";
}) {
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const [busy, setBusy] = useState<CopyAction | null>(null);
  const [copiedAction, setCopiedAction] = useState<CopyAction | null>(null);
  const [redact, setRedact] = useState(false);

  const groupRef = useRef<HTMLSpanElement | null>(null);
  const caretRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const fullResultRef = useRef<{ id: string; value: unknown } | null>(null);
  const timerRef = useRef<number | null>(null);
  const mountedRef = useRef(true);

  const sensitiveFields = useMemo(
    () => findSensitiveFields(record.params),
    [record.params],
  );
  const sensitiveKeys = useMemo(
    () => [...new Set(sensitiveFields.map((hit) => hit.key))].slice(0, 3),
    [sensitiveFields],
  );
  const frameNote = record.frames?.truncated
    ? "部分原始帧未保存，复制时按记录重建"
    : record.frames?.request || record.frames?.response
      ? "请求/响应使用原始帧"
      : "请求/响应按记录重建，id 可能与线上不同";

  const hasResponse =
    record.truncated ||
    (record.result !== null && record.result !== undefined) ||
    Boolean(record.error) ||
    Boolean(record.frames?.response);
  const hasRpcResponse =
    record.method !== "probe" &&
    (record.truncated ||
      (record.result !== null && record.result !== undefined) ||
      record.error?.kind === "rpc_error" ||
      Boolean(record.frames?.response));

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!open) return;

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        groupRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    const onScroll = (event: Event) => {
      const target = event.target;
      if (target instanceof Node && menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onResize = () => setOpen(false);
    const onFocusIn = (event: FocusEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        menuRef.current?.contains(target) ||
        groupRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOpen(false);
      caretRef.current?.focus();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("scroll", onScroll, true);
    document.addEventListener("focusin", onFocusIn, true);
    window.addEventListener("resize", onResize);
    document.addEventListener("keydown", onKeyDown);
    menuRef.current?.focus({ preventScroll: true });

    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("scroll", onScroll, true);
      document.removeEventListener("focusin", onFocusIn, true);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  function openMenu() {
    const group = groupRef.current;
    if (!group) return;
    const rect = group.getBoundingClientRect();
    const width = Math.min(MENU_WIDTH, window.innerWidth - 16);
    const left = Math.max(
      8,
      Math.min(rect.right - width, window.innerWidth - width - 8),
    );
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const spaceAbove = rect.top - 8;
    const openUp = spaceBelow < 300 && spaceAbove > spaceBelow;
    const maxHeight = Math.max(
      120,
      Math.min(360, openUp ? spaceAbove : spaceBelow),
    );
    setPlacement({
      left,
      width,
      maxHeight,
      ...(openUp
        ? { bottom: window.innerHeight - rect.top + 6 }
        : { top: rect.bottom + 6 }),
    });
    setOpen(true);
  }

  function closeMenu(focusCaret: boolean) {
    setOpen(false);
    if (focusCaret) caretRef.current?.focus();
  }

  async function resolveFullResult(): Promise<
    { canceled: true } | { canceled: false; value: unknown | undefined }
  > {
    if (!record.truncated) return { canceled: false, value: undefined };
    if (fullResultRef.current?.id === record.id) {
      return { canceled: false, value: fullResultRef.current.value };
    }
    if (!confirmLargeResultRead(record)) {
      return { canceled: true };
    }
    const text = await readFullResult(record.id);
    let value: unknown = text;
    try {
      value = JSON.parse(text) as unknown;
    } catch {
      // 完整结果文件理论上一定是合法 JSON；保留原文兜底
    }
    fullResultRef.current = { id: record.id, value };
    return { canceled: false, value };
  }

  async function copy(action: CopyAction) {
    if (busy !== null) return;
    setOpen(false);
    setBusy(action);
    try {
      const needsFull =
        action === "response" ||
        action === "response-envelope" ||
        action === "pair-markdown" ||
        action === "pair-json" ||
        action === "text-digest";
      let fullResult: unknown | undefined;
      if (needsFull) {
        const resolved = await resolveFullResult();
        if (resolved.canceled) return;
        fullResult = resolved.value;
      }

      let text: string | null = null;
      switch (action) {
        case "request":
          text = buildRequestText(record, record.frames, redact);
          break;
        case "response":
          text = buildResponseBodyText(record, fullResult, redact);
          break;
        case "response-envelope":
          text = buildResponseEnvelopeText(
            record,
            fullResult,
            record.frames,
            redact,
          );
          break;
        case "params":
          text = buildRequestParamsText(record, redact);
          break;
        case "pair-markdown":
          text = buildPairMarkdown(record, fullResult, record.frames, redact);
          break;
        case "pair-json":
          text = buildPairJson(record, fullResult, record.frames, redact);
          break;
        case "text-digest":
          text = buildTextDigest(record, fullResult, redact);
          break;
        case "http-skeleton": {
          const server = await getServer(record.server_id);
          const requestText = buildRequestText(record, record.frames, true);
          text = requestText
            ? buildHttpSkeleton(server, requestText, record.protocol_version)
            : null;
          break;
        }
      }

      if (!text) {
        toast.error("没有可复制的内容", {
          message:
            action === "response-envelope"
              ? "当前记录没有标准 JSON-RPC 响应。"
              : action === "http-skeleton"
                ? "仅 Streamable HTTP 且非探测记录可生成请求骨架。"
                : action === "text-digest"
                  ? "当前结果没有可复制的文本内容。"
                  : "本次调用没有可复制的请求或响应。",
        });
        return;
      }

      await writeClipboardText(text);
      setCopiedAction(action);
      if (action !== "response") {
        toast.success("已复制", { message: ACTION_LABEL[action] });
      }
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      timerRef.current = window.setTimeout(() => setCopiedAction(null), 1400);
    } catch (error) {
      const isReadFailure =
        action === "response" ||
        action === "response-envelope" ||
        action === "pair-markdown" ||
        action === "pair-json";
      const info = toErrorInfo(error);
      const title =
        action === "http-skeleton"
          ? "生成 HTTP 请求骨架失败"
          : isReadFailure
            ? "读取或复制结果失败"
            : "复制失败";
      toast.error(title, {
        message: info.message,
        detail: info.kind ? info : undefined,
        action: {
          label: "重试",
          run: () => {
            if (!mountedRef.current) return;
            void copy(action);
          },
        },
      });
    } finally {
      setBusy(null);
    }
  }

  const entries: MenuEntry[] = [
    {
      action: "request",
      label: "复制请求",
      hint:
        record.method === "probe"
          ? "探测记录没有标准 JSON-RPC 请求"
          : "完整 JSON-RPC 请求",
      disabled: record.method === "probe",
    },
    {
      action: "response",
      label: "复制响应",
      hint: hasResponse
        ? record.truncated
          ? `完整结果约 ${formatBytes(record.size_bytes)}`
          : "结果 body；无结果时为错误详情"
        : "本次调用没有响应",
      disabled: !hasResponse,
    },
    {
      action: "pair-markdown",
      label: "复制请求和响应",
      hint: "Markdown 报告，适合贴 issue / 聊天",
      disabled: false,
    },
    {
      action: "pair-json",
      label: "复制请求和响应（JSON）",
      hint: "适合脚本或后续导入",
      disabled: false,
    },
    {
      action: "params",
      label: "复制请求参数",
      hint: record.method === "probe" ? "探测记录没有请求参数" : "只有 params",
      disabled: record.method === "probe",
      separator: true,
    },
    {
      action: "response-envelope",
      label: "复制响应（JSON-RPC）",
      hint: hasRpcResponse
        ? "完整 jsonrpc / id / result 或 error"
        : "当前记录没有标准 JSON-RPC 响应",
      disabled: !hasRpcResponse,
    },
    {
      action: "http-skeleton",
      label: "复制 HTTP 请求骨架",
      hint:
        record.transport === "streamable_http"
          ? "仅 Streamable HTTP；头/URL/body 已脱敏"
          : "仅支持 Streamable HTTP",
      disabled:
        record.transport !== "streamable_http" ||
        record.method === "probe" ||
        record.error?.kind === "validation",
      separator: true,
    },
    {
      action: "text-digest",
      label: "复制文本内容",
      hint: "跳过图片/音频 base64，只保留文本与占位符",
      disabled: record.result === null && !record.truncated,
    },
  ];

  const readingFull =
    busy !== null &&
    record.truncated &&
    (busy === "response" ||
      busy === "response-envelope" ||
      busy === "pair-markdown" ||
      busy === "pair-json" ||
      busy === "text-digest");
  const primaryAvailable =
    primaryAction === "request" ? record.method !== "probe" : hasResponse;
  const primaryLabel = primaryAction === "request" ? "复制请求" : "复制响应";
  const mainLabel =
    busy !== null
      ? readingFull
        ? "读取中…"
        : "复制中…"
      : copiedAction === primaryAction
        ? "已复制"
        : primaryLabel;

  function handleMenuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (
      event.key !== "ArrowDown" &&
      event.key !== "ArrowUp" &&
      event.key !== "Home" &&
      event.key !== "End"
    ) {
      return;
    }
    event.preventDefault();
    const nodes = menuRef.current?.querySelectorAll<HTMLButtonElement>(
      '[role="menuitem"]:not(:disabled), [role="menuitemcheckbox"]:not(:disabled)',
    );
    if (!nodes || nodes.length === 0) return;
    const items = Array.from(nodes);
    const active = document.activeElement;
    const current =
      active instanceof HTMLButtonElement ? items.indexOf(active) : -1;
    let next = 0;
    if (event.key === "ArrowDown") {
      next = current < 0 ? 0 : (current + 1) % items.length;
    } else if (event.key === "ArrowUp") {
      next = current <= 0 ? items.length - 1 : current - 1;
    } else if (event.key === "End") {
      next = items.length - 1;
    }
    items[next]?.focus();
  }

  return (
    <span ref={groupRef} className="inline-flex shrink-0">
      <button
        type="button"
        className="whitespace-nowrap rounded-l border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
        disabled={busy !== null || !primaryAvailable}
        title={
          primaryAction === "request"
            ? record.method === "probe"
              ? "探测记录没有标准 JSON-RPC 请求"
              : "复制请求（JSON-RPC）"
            : hasResponse
              ? record.truncated
                ? `复制响应（将读取完整结果，约 ${formatBytes(record.size_bytes)}）`
                : "复制响应（结果 body；无结果时为错误详情）"
              : "本次调用没有响应"
        }
        onClick={() => void copy(primaryAction)}
      >
        {mainLabel}
      </button>
      <button
        ref={caretRef}
        type="button"
        className="rounded-r border border-l-0 border-slate-200 px-1.5 py-0.5 text-[11px] text-slate-500 hover:bg-slate-50 disabled:opacity-40"
        aria-label="更多复制选项"
        aria-haspopup="menu"
        aria-expanded={open}
        title="更多复制选项"
        disabled={busy !== null}
        onClick={() => {
          if (open) closeMenu(true);
          else openMenu();
        }}
      >
        ▾
      </button>

      {open && placement
        ? createPortal(
            <div
              ref={menuRef}
              role="menu"
              aria-label="复制调用内容"
              aria-orientation="vertical"
              tabIndex={-1}
              className="fixed z-50 overflow-y-auto rounded-lg border border-slate-200 bg-white py-1 text-xs shadow-xl outline-none"
              style={{
                left: placement.left,
                width: placement.width,
                maxHeight: placement.maxHeight,
                top: placement.top,
                bottom: placement.bottom,
              }}
              onKeyDown={handleMenuKeyDown}
            >
              <div className="px-3 py-1 text-[10px] font-medium text-slate-400">
                复制调用内容
              </div>
              <div className="border-y border-slate-100">
                <button
                  type="button"
                  role="menuitemcheckbox"
                  aria-checked={redact}
                  aria-label="复制时脱敏"
                  title="替换常见 token/password/authorization 字段；不保证完全"
                  className="flex w-full items-start gap-2 px-3 py-2 text-left hover:bg-slate-50"
                  onClick={() => setRedact((value) => !value)}
                >
                  <span
                    className={`mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border text-[9px] ${
                      redact
                        ? "border-slate-700 bg-slate-700 text-white"
                        : "border-slate-300 text-transparent"
                    }`}
                    aria-hidden="true"
                  >
                    ✓
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-slate-700">
                      复制时脱敏（常见字段）
                    </span>
                    <span className="block text-[10px] text-slate-400">
                      不保证完全；HTTP 骨架始终脱敏
                    </span>
                  </span>
                </button>
              </div>
              {entries.map((entry) => (
                <div key={entry.action} role="none">
                  {entry.separator ? (
                    <div className="my-1 border-t border-slate-100" />
                  ) : null}
                  <button
                    type="button"
                    role="menuitem"
                    aria-label={entry.label}
                    disabled={entry.disabled || busy !== null}
                    title={entry.hint}
                    className="flex w-full items-start gap-2 px-3 py-2 text-left hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-40"
                    onClick={() => void copy(entry.action)}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block text-slate-700">
                        {entry.label}
                      </span>
                      <span className="block text-[10px] text-slate-400">
                        {entry.hint}
                      </span>
                    </span>
                  </button>
                </div>
              ))}
              <div className="mt-1 space-y-0.5 border-t border-slate-100 px-3 py-1.5 text-[10px] leading-4">
                <div className="text-slate-400">{frameNote}</div>
                {redact ? (
                  <div className="text-emerald-600">
                    已开启脱敏（常见字段）；原始数据仍按原文保存
                  </div>
                ) : sensitiveFields.length > 0 ? (
                  <div className="text-amber-600">
                    参数包含 {sensitiveFields.length} 个敏感字段（
                    {sensitiveKeys.join("、")}
                    {sensitiveFields.length > sensitiveKeys.length ? " 等" : ""}
                    ），分享前请检查
                  </div>
                ) : (
                  <div className="text-slate-400">
                    分享前请检查参数中的敏感信息
                  </div>
                )}
              </div>
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
