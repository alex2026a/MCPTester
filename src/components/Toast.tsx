/**
 * Toast 容器（v0.1.1）：右下角堆叠，失败常驻 + 「详情」弹窗。
 */

import { useEffect, useState } from "react";
import { ErrorDetailDialog } from "./ErrorDetailDialog";
import type { ErrorInfo } from "../lib/errors";
import { useToastStore, type ToastItem } from "../stores/toast";

const LEVEL_STYLE: Record<ToastItem["level"], { border: string; badge: string; label: string }> = {
  success: { border: "border-emerald-200", badge: "bg-emerald-500", label: "成功" },
  error: { border: "border-red-200", badge: "bg-red-500", label: "失败" },
  info: { border: "border-slate-200", badge: "bg-slate-400", label: "提示" },
};

const AUTO_DISMISS_MS = 4000;

export function ToastContainer() {
  const toasts = useToastStore((state) => state.toasts);
  const dismiss = useToastStore((state) => state.dismiss);
  const [detail, setDetail] = useState<ErrorInfo | null>(null);

  // 成功 / 信息：自动消失
  useEffect(() => {
    const timers = toasts
      .filter((item) => !item.sticky)
      .map((item) => window.setTimeout(() => dismiss(item.id), AUTO_DISMISS_MS));
    return () => {
      for (const timer of timers) window.clearTimeout(timer);
    };
  }, [toasts, dismiss]);

  if (toasts.length === 0 && !detail) return null;

  return (
    <>
      <div className="pointer-events-none fixed bottom-4 right-4 z-40 flex w-96 max-w-[calc(100vw-2rem)] flex-col gap-2">
        {toasts.map((item) => {
          const style = LEVEL_STYLE[item.level];
          return (
            <div
              key={item.id}
              className={`pointer-events-auto rounded-lg border ${style.border} bg-white p-3 shadow-lg`}
            >
              <div className="flex items-start gap-2">
                <span className={`mt-1 h-2 w-2 shrink-0 rounded-full ${style.badge}`} />
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-medium text-slate-800">{item.title}</div>
                  {item.message ? (
                    <div className="mt-0.5 line-clamp-3 break-words text-xs text-slate-500">
                      {item.message}
                    </div>
                  ) : null}
                  <div className="mt-1.5 flex items-center gap-2">
                    {item.detail ? (
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-1.5 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                        onClick={() => setDetail(item.detail ?? null)}
                      >
                        详情
                      </button>
                    ) : null}
                    {item.action ? (
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-1.5 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                        onClick={() => {
                          item.action?.run();
                          dismiss(item.id);
                        }}
                      >
                        {item.action.label}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="ml-auto text-[11px] text-slate-400 hover:text-slate-600"
                      onClick={() => dismiss(item.id)}
                    >
                      关闭
                    </button>
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {detail ? (
        <ErrorDetailDialog info={detail} onClose={() => setDetail(null)} />
      ) : null}
    </>
  );
}
