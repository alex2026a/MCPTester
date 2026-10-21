/**
 * 服务器配置 JSON 弹窗（v0.1.1）：以 Claude 格式查看 / 复制（敏感值为占位符）。
 */

import { useQuery } from "@tanstack/react-query";
import { JsonViewer } from "./JsonViewer";
import { getServerJson } from "../api/servers";

export function ServerConfigDialog({
  serverId,
  serverName,
  onClose,
}: {
  serverId: string;
  serverName: string;
  onClose: () => void;
}) {
  const query = useQuery({
    queryKey: ["server-json", serverId],
    queryFn: () => getServerJson(serverId),
  });

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/30 p-6"
      role="dialog"
      aria-modal="true"
      onClick={onClose}
    >
      <div
        className="flex max-h-[85vh] w-[680px] max-w-full flex-col rounded-lg bg-white p-4 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-3">
          <h3 className="text-sm font-semibold text-slate-800">
            配置 JSON · {serverName}
          </h3>
          <span className="text-[11px] text-slate-400">
            Claude 格式；敏感值为占位符
          </span>
          <button
            type="button"
            className="ml-auto rounded border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-50"
            onClick={onClose}
          >
            关闭
          </button>
        </div>
        <div className="mt-3 min-h-0 flex-1 overflow-hidden">
          {query.isLoading ? (
            <div className="text-xs text-slate-400">加载中…</div>
          ) : query.isError ? (
            <div className="text-xs text-red-500">加载失败：{String(query.error)}</div>
          ) : (
            <JsonViewer
              text={query.data ?? ""}
              defaultMode="text"
              contentClassName="max-h-[60vh] overflow-auto"
            />
          )}
        </div>
      </div>
    </div>
  );
}
