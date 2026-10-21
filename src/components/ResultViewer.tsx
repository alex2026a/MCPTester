/**
 * 结果视图（v0.1.3）
 *
 * - 元信息条（状态 / 耗时 / 协议 / 传输 / 时间 / 大小）；
 * - 结构化错误单独展示（不再与报文重复）；
 * - MessageViewer 三页签（渲染 / 结构 / 原文）+ 截断结果出口。
 */

import { useCallback, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { MessageViewer } from "./json/MessageViewer";
import { formatBytes, formatDateTime } from "./json/format";
import { getFullResultPath, readFullResult } from "../api/history";
import { toast } from "../stores/toast";
import type { CallRecord } from "../types/models";

const STATUS: Record<string, { text: string; className: string }> = {
  ok: { text: "成功", className: "text-emerald-600" },
  error: { text: "失败", className: "text-red-600" },
  canceled: { text: "已取消", className: "text-slate-500" },
};

export function ResultViewer({ record }: { record: CallRecord }) {
  const isError = record.status === "error";
  const status = STATUS[record.status] ?? { text: record.status, className: "text-slate-500" };
  const [fullText, setFullText] = useState<string | null>(null);
  const [loadingFull, setLoadingFull] = useState(false);

  const openFull = useCallback(() => {
    setLoadingFull(true);
    void readFullResult(record.id)
      .then((text) => setFullText(text))
      .catch((error) => toast.fromError(error, "读取完整结果失败"))
      .finally(() => setLoadingFull(false));
  }, [record.id]);

  const reveal = useCallback(() => {
    void getFullResultPath(record.id)
      .then((path) => {
        if (!path) {
          toast.error("未找到完整结果文件", { message: "该记录可能未被截断，或落盘文件已被清理。" });
          return;
        }
        return revealItemInDir(path);
      })
      .catch((error) => toast.fromError(error, "打开所在文件夹失败"));
  }, [record.id]);

  const hasResult = record.result !== null && record.result !== undefined;

  return (
    <div
      className={`rounded-lg border p-3 text-sm ${
        isError ? "border-red-200 bg-red-50" : "border-slate-200 bg-white"
      }`}
    >
      <div className="mb-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-slate-500">
        <span className={status.className}>{status.text}</span>
        <span className="text-slate-300">·</span>
        <span>耗时 {record.duration_ms}ms</span>
        <span className="text-slate-300">·</span>
        <span>{record.protocol_version || "未知协议"}</span>
        <span className="text-slate-300">·</span>
        <span>{record.transport}</span>
        <span className="text-slate-300">·</span>
        <span>{formatDateTime(record.created_at)}</span>
        <span className="ml-auto flex items-center gap-1.5">
          {record.truncated ? (
            <span className="rounded bg-amber-100 px-1 py-0.5 text-[10px] text-amber-700">
              结果已截断
            </span>
          ) : null}
          <span>{formatBytes(record.size_bytes)}</span>
        </span>
      </div>

      {isError && record.error ? (
        <div className="mb-2 flex items-start gap-2 rounded border border-red-200 bg-white/70 p-2 text-xs text-red-700">
          {record.error.kind ? (
            <span className="shrink-0 rounded bg-red-100 px-1 py-0.5 font-mono text-[10px]">
              {record.error.kind}
            </span>
          ) : null}
          <span className="break-all">{record.error.message ?? "调用失败"}</span>
        </div>
      ) : null}

      {record.truncated ? (
        <div className="mb-2 flex flex-wrap items-center gap-2 rounded border border-amber-200 bg-amber-50 px-2 py-1.5 text-[11px] text-amber-700">
          <span>结果超过 1MB，已截断入库；完整内容已落盘。</span>
          <button
            type="button"
            className="rounded border border-amber-300 bg-white px-1.5 py-0.5 hover:bg-amber-100 disabled:opacity-50"
            disabled={loadingFull}
            onClick={openFull}
          >
            {loadingFull ? "读取中…" : "打开完整结果"}
          </button>
          <button
            type="button"
            className="rounded border border-amber-300 bg-white px-1.5 py-0.5 hover:bg-amber-100"
            onClick={reveal}
          >
            在文件夹中显示
          </button>
        </div>
      ) : null}

      {hasResult ? (
        <MessageViewer
          key={record.id}
          value={record.result}
          method={record.method}
          sizeBytes={record.size_bytes}
          truncated={record.truncated}
          onOpenFullResult={record.truncated ? openFull : undefined}
          onRevealFullResult={record.truncated ? reveal : undefined}
          contentClassName="max-h-96 overflow-auto"
        />
      ) : record.status === "canceled" ? (
        <div className="text-xs text-slate-400">调用已取消，无结果。</div>
      ) : null}

      {fullText !== null ? (
        <FullResultDialog
          text={fullText}
          onClose={() => setFullText(null)}
          onReveal={reveal}
        />
      ) : null}
    </div>
  );
}

function FullResultDialog({
  text,
  onClose,
  onReveal,
}: {
  text: string;
  onClose: () => void;
  onReveal: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/30 p-6"
      role="dialog"
      aria-modal="true"
      onClick={onClose}
    >
      <div
        className="flex max-h-[88vh] w-[1000px] max-w-full flex-col rounded-lg bg-white p-3 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="mb-2 flex items-center gap-2">
          <h3 className="text-sm font-semibold text-slate-800">完整结果</h3>
          <span className="text-[11px] text-slate-400">
            {formatBytes(new Blob([text]).size)} · {text.length.toLocaleString()} 字符
          </span>
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-[11px] text-slate-600 hover:bg-slate-50"
              onClick={onReveal}
            >
              在文件夹中显示
            </button>
            <button
              type="button"
              className="rounded bg-slate-800 px-2 py-1 text-[11px] text-white hover:bg-slate-700"
              onClick={onClose}
            >
              关闭
            </button>
          </div>
        </div>
        <div className="min-h-0 flex-1">
          <MessageViewer
            text={text}
            defaultTab="raw"
            tabs={["tree", "raw"]}
            contentClassName="max-h-[70vh] overflow-auto"
          />
        </div>
      </div>
    </div>
  );
}
