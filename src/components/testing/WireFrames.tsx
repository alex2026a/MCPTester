/**
 * 测试报告报文查看（v0.2.2）
 *
 * 使用 pairWireEntries 按 JSON-RPC id 配对请求/响应；主从布局，只渲染当前选中
 * 交互的两个 MessageViewer，避免一次挂载大量查看器。
 */

import { useMemo, useState } from "react";
import type { WireEntry } from "../../api/wire";
import { pairWireEntries, type WireInteraction } from "../../lib/wire";
import { MessageViewer } from "../json/MessageViewer";
import { formatBytes, formatClock, formatDuration } from "../json/format";

export function WireFrames({ entries }: { entries: WireEntry[] }) {
  const sessions = useMemo(
    () =>
      pairWireEntries(entries).filter((item) => item.request || item.response),
    [entries],
  );
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const selected =
    sessions.find((item) => item.key === selectedKey) ?? sessions[0] ?? null;

  if (sessions.length === 0) {
    return <div className="text-[11px] text-slate-400">无请求/响应报文。</div>;
  }

  return (
    <div className="flex flex-col gap-2 md:flex-row">
      <div className="max-h-64 shrink-0 overflow-auto rounded border border-slate-100 md:w-72">
        {sessions.map((item) => (
          <WireRow
            key={item.key}
            item={item}
            selected={item.key === selected?.key}
            onSelect={() => setSelectedKey(item.key)}
          />
        ))}
      </div>
      <div className="min-w-0 flex-1">
        {selected ? <WireDetail item={selected} /> : null}
      </div>
    </div>
  );
}

function WireRow({
  item,
  selected,
  onSelect,
}: {
  item: WireInteraction;
  selected: boolean;
  onSelect: () => void;
}) {
  const start = item.request ?? item.response;
  if (!start) return null;
  const direction = start.direction === "client_to_server" ? "→" : "←";
  const id = item.request?.rpc_id ?? item.response?.rpc_id;
  const label = item.request?.method ?? "response";
  const duration = item.response?.duration_ms ?? item.request?.duration_ms ?? null;
  const bytes = (item.request?.bytes ?? 0) + (item.response?.bytes ?? 0);
  const truncated = item.request?.truncated || item.response?.truncated;
  const failed = item.response?.is_error ?? false;
  const pending = Boolean(item.request && !item.response);

  return (
    <button
      type="button"
      className={`flex w-full items-center gap-1.5 border-b border-slate-50 px-2 py-1.5 text-left font-mono text-[10px] last:border-b-0 ${
        selected ? "bg-slate-100" : "hover:bg-slate-50"
      }`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="w-[62px] shrink-0 text-slate-400">
        {formatClock(start.at)}
      </span>
      <span
        className={`w-3 shrink-0 ${
          direction === "→" ? "text-blue-500" : "text-emerald-600"
        }`}
      >
        {direction}
      </span>
      <span className="min-w-0 flex-1 truncate text-slate-700" title={label}>
        {label}
      </span>
      {id ? <span className="w-10 shrink-0 truncate text-slate-400">#{id}</span> : null}
      <span
        className={`w-9 shrink-0 ${
          failed ? "text-red-600" : pending ? "text-amber-600" : "text-emerald-600"
        }`}
      >
        {failed ? "error" : pending ? "在途" : "ok"}
      </span>
      {truncated ? <span className="shrink-0 text-amber-600">截断</span> : null}
      <span className="w-12 shrink-0 text-right text-slate-400">
        {formatDuration(duration)}
      </span>
      <span className="w-12 shrink-0 text-right text-slate-400">
        {formatBytes(bytes)}
      </span>
    </button>
  );
}

function WireDetail({ item }: { item: WireInteraction }) {
  return (
    <div className="grid gap-2 lg:grid-cols-2">
      <FramePane title="请求" entry={item.request} />
      <FramePane title="响应" entry={item.response} />
    </div>
  );
}

function FramePane({ title, entry }: { title: string; entry?: WireEntry }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 flex flex-wrap items-center gap-2 text-[10px] text-slate-500">
        <span className="font-medium text-slate-600">{title}</span>
        {entry ? (
          <>
            <span className="font-mono">{entry.method ?? entry.kind}</span>
            {entry.rpc_id ? (
              <span className="font-mono">#{entry.rpc_id}</span>
            ) : null}
            <span>{formatBytes(entry.bytes)}</span>
            {entry.truncated ? (
              <span className="text-amber-600">已截断</span>
            ) : null}
          </>
        ) : (
          <span className="text-slate-400">未收到</span>
        )}
      </div>
      {entry ? (
        <MessageViewer
          key={`${title}-${entry.seq}`}
          value={entry.payload}
          defaultTab="tree"
          tabs={["tree", "raw"]}
          copyLabel={title === "请求" ? "复制请求" : "复制响应"}
          contentClassName="max-h-60 overflow-auto"
        />
      ) : (
        <div className="rounded border border-slate-100 bg-slate-50 px-2 py-3 text-[11px] text-slate-400">
          {title === "响应" ? "未收到对应响应（可能仍在途或已取消）。" : "无请求帧。"}
        </div>
      )}
    </div>
  );
}
