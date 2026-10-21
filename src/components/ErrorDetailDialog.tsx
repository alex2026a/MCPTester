/**
 * 结构化错误详情弹窗（v0.1.1）
 *
 * 展示：人话标题 / 消息 / 修复建议 / 原始技术细节（可复制）。
 */

import { useState } from "react";
import type { ErrorInfo } from "../lib/errors";

export function ErrorDetailDialog({
  info,
  onClose,
}: {
  info: ErrorInfo;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);

  async function copyAll() {
    const text = [
      `【${info.title}】`,
      info.message,
      info.hint ? `建议：${info.hint}` : null,
      info.raw ? `原始细节：\n${info.raw}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    try {
      await navigator.clipboard?.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // 剪贴板不可用：忽略（用户可手动选择文本）
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/30 p-6"
      role="dialog"
      aria-modal="true"
      onClick={onClose}
    >
      <div
        className="max-h-[80vh] w-[600px] max-w-full overflow-y-auto rounded-lg bg-white p-4 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3">
          <h3 className="text-sm font-semibold text-slate-800">
            {info.title}
            {info.kind ? (
              <span className="ml-2 rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[10px] font-normal text-slate-400">
                {info.kind}
              </span>
            ) : null}
          </h3>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-0.5 text-xs text-slate-600 hover:bg-slate-50"
              onClick={() => void copyAll()}
            >
              {copied ? "已复制" : "复制全部"}
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-0.5 text-xs text-slate-600 hover:bg-slate-50"
              onClick={onClose}
            >
              关闭
            </button>
          </div>
        </div>

        <p className="mt-3 whitespace-pre-wrap break-words text-xs leading-5 text-slate-700">
          {info.message}
        </p>

        {info.hint ? (
          <div className="mt-3 rounded border border-amber-200 bg-amber-50 p-2 text-xs leading-5 text-amber-800">
            <span className="font-medium">建议：</span>
            {info.hint}
          </div>
        ) : null}

        {info.raw ? (
          <details className="mt-3" open>
            <summary className="cursor-pointer text-xs text-slate-500">
              原始技术细节
            </summary>
            <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap break-all rounded bg-slate-50 p-2 font-mono text-[11px] leading-5 text-slate-600">
              {info.raw}
            </pre>
          </details>
        ) : null}
      </div>
    </div>
  );
}
