/**
 * 导入服务器配置弹窗（v0.1.1）：文件选择 / 粘贴 JSON 两种方式。
 */

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  importServers,
  importServersFromFile,
  type ImportReport,
} from "../api/servers";
import { toast } from "../stores/toast";

export function ImportDialog({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported: (report: ImportReport) => void;
}) {
  const [tab, setTab] = useState<"file" | "paste">("file");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);

  async function pickFile() {
    const picked = await open({
      multiple: false,
      filters: [
        { name: "JSON", extensions: ["json"] },
        { name: "所有文件", extensions: ["*"] },
      ],
    });
    if (!picked || Array.isArray(picked)) return;
    setBusy(true);
    try {
      const report = await importServersFromFile(picked);
      onImported(report);
    } catch (error) {
      toast.fromError(error, "导入失败");
    } finally {
      setBusy(false);
    }
  }

  async function importPasted() {
    if (!text.trim()) {
      toast.error("请先粘贴配置 JSON", { sticky: false });
      return;
    }
    setBusy(true);
    try {
      const report = await importServers(text);
      onImported(report);
    } catch (error) {
      toast.fromError(error, "导入失败");
    } finally {
      setBusy(false);
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
        className="w-[560px] max-w-full rounded-lg bg-white p-4 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-3">
          <h3 className="text-sm font-semibold text-slate-800">
            导入服务器配置
          </h3>
          <div className="flex items-center gap-1">
            <button
              type="button"
              className={`rounded px-2 py-0.5 text-xs ${
                tab === "file"
                  ? "bg-slate-800 text-white"
                  : "border border-slate-200 text-slate-600 hover:bg-slate-50"
              }`}
              onClick={() => setTab("file")}
            >
              从文件
            </button>
            <button
              type="button"
              className={`rounded px-2 py-0.5 text-xs ${
                tab === "paste"
                  ? "bg-slate-800 text-white"
                  : "border border-slate-200 text-slate-600 hover:bg-slate-50"
              }`}
              onClick={() => setTab("paste")}
            >
              粘贴 JSON
            </button>
          </div>
          <button
            type="button"
            className="ml-auto rounded border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-50"
            onClick={onClose}
          >
            关闭
          </button>
        </div>

        {tab === "file" ? (
          <div className="mt-4 space-y-3">
            <p className="text-xs text-slate-500">
              选择 Claude Desktop / Cursor（mcpServers）或 VS
              Code（servers）配置文件 （`.json`）。敏感项按「键名优先 +
              值模式兜底」识别；安装版入凭据库， 便携版随数据目录明文保存。
            </p>
            <button
              type="button"
              className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
              disabled={busy}
              onClick={() => void pickFile()}
            >
              {busy ? "导入中…" : "选择文件并导入"}
            </button>
          </div>
        ) : (
          <div className="mt-4 space-y-3">
            <textarea
              className="h-48 w-full rounded border border-slate-200 p-2 font-mono text-xs"
              placeholder={
                '{"mcpServers": { "demo": { "url": "http://127.0.0.1:8000/mcp" } }}'
              }
              value={text}
              onChange={(event) => setText(event.target.value)}
            />
            <button
              type="button"
              className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
              disabled={busy}
              onClick={() => void importPasted()}
            >
              {busy ? "导入中…" : "导入"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
