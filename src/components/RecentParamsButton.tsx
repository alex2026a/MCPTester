import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { getCallParams, listRecentCallParams } from "../api/history";
import { replayArguments } from "../lib/replay";
import { toast } from "../stores/toast";
import {
  formatClock,
  formatDateTime,
  formatDuration,
} from "./json/format";
import type { RecentCallParam } from "../types/models";

export type RecentParamsContext = {
  serverId: string;
  method: "tools_call" | "prompts_get";
  target: string;
};

type Props = {
  context: RecentParamsContext;
  onApply: (
    params: Record<string, unknown>,
    item: RecentCallParam,
    context: RecentParamsContext,
  ) => void;
};

type Placement = {
  left: number;
  width: number;
  maxHeight: number;
  top?: number;
  bottom?: number;
};

const PANEL_WIDTH = 420;

function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return formatClock(iso).slice(0, 8);
  }
  return formatDateTime(iso);
}

function statusLabel(status: RecentCallParam["status"]) {
  switch (status) {
    case "ok":
      return { text: "成功", className: "text-emerald-600" };
    case "error":
      return { text: "失败", className: "text-red-600" };
    default:
      return { text: "取消", className: "text-slate-500" };
  }
}

/** 接口页“历史参数”按钮与浮层。只回填参数，不自动发送。 */
export function RecentParamsButton({ context, onApply }: Props) {
  const navigate = useNavigate();
  const panelId = useId();
  const [open, setOpen] = useState(false);
  const [includeTest, setIncludeTest] = useState(false);
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const mountedRef = useRef(false);
  const requestSeq = useRef(0);
  const wasOpenRef = useRef(false);

  const query = useQuery({
    queryKey: [
      "recent-params",
      context.serverId,
      context.method,
      context.target,
      includeTest,
    ],
    queryFn: () =>
      listRecentCallParams({
        serverId: context.serverId,
        method: context.method,
        target: context.target,
        includeTest,
      }),
    enabled: open,
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestSeq.current += 1;
    };
  }, []);

  const closePanel = useCallback(
    (focusButton: boolean) => {
      if (focusButton && wasOpenRef.current) {
        buttonRef.current?.focus();
      }
      wasOpenRef.current = false;
      requestSeq.current += 1;
      setLoadingId(null);
      setOpen(false);
    },
    [],
  );

  useEffect(() => {
    if (!open) return;

    const onScroll = (event: Event) => {
      const target = event.target;
      if (target instanceof Node && panelRef.current?.contains(target)) {
        return;
      }
      closePanel(false);
    };
    const onResize = () => closePanel(false);
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        panelRef.current?.contains(target) ||
        buttonRef.current?.contains(target)
      ) {
        return;
      }
      closePanel(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closePanel(true);
      }
    };

    document.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);

    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      let first = true;
      observer = new ResizeObserver(() => {
        if (first) {
          first = false;
          return;
        }
        closePanel(false);
      });
      observer.observe(document.body);
    }

    // 让 Tab 从面板容器进入列表，而不是从页面末尾开始
    panelRef.current?.focus();

    return () => {
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
      observer?.disconnect();
    };
  }, [closePanel, open]);

  function openPanel() {
    const button = buttonRef.current;
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const width = Math.min(PANEL_WIDTH, window.innerWidth - 16);
    const left = Math.max(
      8,
      Math.min(rect.right - width, window.innerWidth - width - 8),
    );
    const spaceAbove = rect.top - 8;
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const openUp = spaceAbove >= Math.min(320, spaceBelow);
    const space = openUp ? spaceAbove : spaceBelow;
    const maxHeight = Math.max(96, Math.min(520, space));
    setPlacement({
      left,
      width,
      maxHeight,
      ...(openUp
        ? { bottom: window.innerHeight - rect.top + 8 }
        : { top: rect.bottom + 8 }),
    });
    wasOpenRef.current = true;
    setOpen(true);
  }

  async function applyItem(item: RecentCallParam) {
    if (loadingId) return;
    const seq = ++requestSeq.current;
    setLoadingId(item.id);
    try {
      const envelope = await getCallParams(item.id);
      if (!mountedRef.current || seq !== requestSeq.current) return;
      onApply(replayArguments(envelope), item, context);
      closePanel(true);
    } catch (error) {
      if (mountedRef.current && seq === requestSeq.current) {
        toast.fromError(error, "读取历史参数失败");
        void query.refetch();
      }
    } finally {
      if (mountedRef.current && seq === requestSeq.current) {
        setLoadingId(null);
      }
    }
  }

  function openHistory() {
    const params = new URLSearchParams({
      server_id: context.serverId,
      method: context.method,
      target: context.target,
      origin: "all",
    });
    closePanel(false);
    navigate(`/history?${params.toString()}`);
  }

  const items = query.data ?? [];

  return (
    <span className="relative inline-flex">
      <button
        ref={buttonRef}
        type="button"
        className="rounded border border-slate-200 px-2 py-1.5 text-xs text-slate-600 hover:bg-slate-50"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={panelId}
        title="加载本接口最近发送过的参数"
        onClick={() => {
          if (open) closePanel(true);
          else openPanel();
        }}
      >
        历史参数 ▾
      </button>

      {open && placement
        ? createPortal(
            <div
              ref={panelRef}
              id={panelId}
              role="dialog"
              aria-label="最近参数"
              tabIndex={-1}
              className="fixed z-50 flex flex-col overflow-hidden rounded-lg border border-slate-200 bg-white shadow-xl outline-none"
              style={{
                left: placement.left,
                width: placement.width,
                maxHeight: placement.maxHeight,
                top: placement.top,
                bottom: placement.bottom,
              }}
            >
              <div className="flex shrink-0 items-center gap-2 border-b border-slate-100 px-3 py-2 text-xs text-slate-500">
                <span className="font-medium text-slate-700">最近参数</span>
                <span className="truncate font-mono text-[10px] text-slate-400">
                  {context.target}
                </span>
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto">
                {query.isLoading ? (
                  <div className="p-3 text-xs text-slate-400">读取历史…</div>
                ) : query.isError ? (
                  <div className="space-y-2 p-3 text-xs text-red-500">
                    <div>历史参数加载失败：{String(query.error)}</div>
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-0.5 text-slate-600 hover:bg-slate-50"
                      onClick={() => void query.refetch()}
                    >
                      重试
                    </button>
                  </div>
                ) : items.length === 0 ? (
                  <div className="p-3 text-xs text-slate-400">
                    这个接口还没有调用记录，发送一次后这里会自动出现。
                  </div>
                ) : (
                  items.map((item) => {
                    const status = statusLabel(item.status);
                    const loading = loadingId === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        disabled={loadingId !== null}
                        className="block w-full border-b border-slate-100 px-3 py-2 text-left text-xs last:border-b-0 hover:bg-slate-50 disabled:opacity-60"
                        onClick={() => void applyItem(item)}
                      >
                        <div className="flex items-center gap-2">
                          <span className="shrink-0 text-slate-400">
                            {formatWhen(item.created_at)}
                          </span>
                          <span className={`shrink-0 ${status.className}`}>
                            {status.text}
                          </span>
                          <span className="shrink-0 text-slate-400">
                            {formatDuration(item.duration_ms)}
                          </span>
                          <span className="shrink-0 rounded bg-slate-100 px-1 text-[10px] text-slate-500">
                            {item.origin === "test" ? "测试" : "手工"}
                          </span>
                          {item.repeat_count > 1 ? (
                            <span className="shrink-0 text-slate-400">
                              ×{item.repeat_count}
                            </span>
                          ) : null}
                          {loading ? (
                            <span className="ml-auto text-slate-400">载入中…</span>
                          ) : null}
                        </div>
                        <div className="mt-0.5 truncate font-mono text-[11px] text-slate-600">
                          {item.preview || "无参数"}
                        </div>
                      </button>
                    );
                  })
                )}
              </div>

              <div className="flex shrink-0 flex-wrap items-center gap-2 border-t border-slate-100 px-3 py-2 text-[11px] text-slate-500">
                <label className="flex items-center gap-1">
                  <input
                    type="checkbox"
                    checked={includeTest}
                    onChange={(event) => setIncludeTest(event.target.checked)}
                  />
                  包含测试调用
                </label>
                <button
                  type="button"
                  className="ml-auto text-sky-600 hover:underline"
                  onClick={openHistory}
                >
                  查看该接口全部历史 →
                </button>
              </div>
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
