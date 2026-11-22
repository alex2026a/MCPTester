/**
 * 导出预览弹窗（v0.1.1）：
 * JSON 预览（JsonViewer）→ 原生"保存到…"对话框 → 成功后 toast 提供"打开所在文件夹"。
 */

import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { JsonViewer } from "./JsonViewer";
import { toast } from "../stores/toast";

export function ExportPreviewDialog({
  title,
  defaultFilename,
  load,
  onSave,
  onClose,
  extra,
  filters,
  previewLimit = 1_000_000,
}: {
  title: string;
  defaultFilename: string;
  /** 生成预览内容；在挂载时捕获，切换内容请通过 key 重挂载 */
  load: () => Promise<string>;
  /** 写入用户选择路径（返回最终路径） */
  onSave: (path: string) => Promise<string>;
  onClose: () => void;
  /** 标题行附加内容（如格式选择） */
  extra?: React.ReactNode;
  /** 保存对话框过滤器；默认 JSON */
  filters?: { name: string; extensions: string[] }[];
  /** 预览字符上限；超出时截断展示，保存仍为完整内容 */
  previewLimit?: number;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // 通过 key 重挂载时重新捕获；不在 effect 内 setState，避免级联渲染。
  const [loadOnce] = useState(() => load);

  useEffect(() => {
    let cancelled = false;
    loadOnce()
      .then((text) => {
        if (!cancelled) setContent(text);
      })
      .catch((error) => {
        if (!cancelled) setLoadError(String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [loadOnce]);

  async function handleSave() {
    const path = await save({
      title,
      defaultPath: defaultFilename,
      filters: filters ?? [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return; // 用户取消
    setSaving(true);
    try {
      const savedPath = await onSave(path);
      toast.success("已导出", {
        message: savedPath,
        action: {
          label: "打开所在文件夹",
          run: () => {
            void revealItemInDir(savedPath);
          },
        },
      });
      onClose();
    } catch (error) {
      toast.fromError(error, "导出失败");
    } finally {
      setSaving(false);
    }
  }

  const previewText =
    content !== null && content.length > previewLimit
      ? content.slice(0, previewLimit)
      : content;
  const previewTruncated = content !== null && content.length > previewLimit;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/30 p-6"
      role="dialog"
      aria-modal="true"
      onClick={onClose}
    >
      <div
        className="flex max-h-[85vh] w-[720px] max-w-full flex-col rounded-lg bg-white p-4 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-3">
          <h3 className="text-sm font-semibold text-slate-800">{title}</h3>
          {extra}
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
              disabled={saving || content === null}
              onClick={() => void handleSave()}
            >
              {saving ? "保存中…" : "保存到…"}
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-50"
              onClick={onClose}
            >
              取消
            </button>
          </div>
        </div>

        <div className="mt-3 min-h-0 flex-1 overflow-hidden">
          {loadError ? (
            <div className="text-xs text-red-500">预览失败：{loadError}</div>
          ) : content === null ? (
            <div className="text-xs text-slate-400">生成预览中…</div>
          ) : (
            <div className="space-y-1">
              {previewTruncated ? (
                <div className="text-[11px] text-amber-600">
                  预览已截断（超过 {previewLimit.toLocaleString()}{" "}
                  字符），保存仍导出完整内容。
                </div>
              ) : null}
              <JsonViewer
                text={previewText ?? ""}
                defaultMode="text"
                contentClassName="max-h-[60vh] overflow-auto"
              />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
