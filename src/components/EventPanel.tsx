/**
 * 事件面板（v0.1.3）
 *
 * - 报文 / 通知 / stderr 三个独立页签（各自计数、暂停、清空）；
 * - 报文按 JSON-RPC id 配对成"交互"（请求 + 响应 + 耗时 + 大小 + 状态）；
 * - 主从两栏：左侧会话时间线（可筛选、跟随最新），右侧详情（复用 MessageViewer）；
 * - 报文由后端增量推送（`mcp://wire`），不再 3 秒全量轮询。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  clearWireLog,
  exportWireLog,
  getWireLog,
  listenWire,
  type WireEntry,
} from "../api/wire";
import { MessageViewer } from "./json/MessageViewer";
import { formatBytes, formatClock, formatDuration } from "./json/format";
import { pairWireEntries, type WireInteraction } from "../lib/wire";
import { toast } from "../stores/toast";

interface EventLine {
  seq: number;
  text: string;
  at: string;
}

type PanelTab = "wire" | "notifications" | "stderr";

const MAX_EVENT_LINES = 500;
const MAX_WIRE_ENTRIES = 300;

/** 报文载荷转检索文本（限制长度，避免大结果拖慢筛选） */
function searchable(value: unknown): string {
  if (value === undefined) return "";
  try {
    const text = JSON.stringify(value);
    return text.length > 4000 ? text.slice(0, 4000) : text;
  } catch {
    return "";
  }
}

export function EventPanel({ serverId }: { serverId: string }) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<PanelTab>("wire");
  const [paused, setPaused] = useState(false);
  const [follow, setFollow] = useState(true);

  const [notifications, setNotifications] = useState<EventLine[]>([]);
  const [stderr, setStderr] = useState<EventLine[]>([]);
  const [wire, setWire] = useState<WireEntry[]>([]);
  const [wireLoading, setWireLoading] = useState(false);
  const [wireError, setWireError] = useState<string | null>(null);

  const [keyword, setKeyword] = useState("");
  const [method, setMethod] = useState("");
  const [onlyErrors, setOnlyErrors] = useState(false);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  const queryClient = useQueryClient();
  const pausedRef = useRef(paused);
  const wireListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  const fetchWire = useCallback(async () => {
    setWireLoading(true);
    setWireError(null);
    try {
      const entries = await getWireLog(serverId);
      setWire(entries);
    } catch (error) {
      setWireError(String(error));
    } finally {
      setWireLoading(false);
    }
  }, [serverId]);

  // 首次拉取由用户交互触发（打开面板 / 切到报文页签），其后靠增量推送补齐
  function openPanel(next: boolean) {
    setOpen(next);
    if (next) void fetchWire();
  }

  function selectTab(next: PanelTab) {
    setTab(next);
    setOpen(true);
    if (next === "wire") void fetchWire();
  }

  // 报文增量推送
  useEffect(() => {
    const pending = listenWire((payload) => {
      if (payload.server_id !== serverId || pausedRef.current) return;
      if (payload.dropped) {
        void fetchWire();
        return;
      }
      if (payload.entries.length === 0) return;
      setWire((previous) => {
        const lastSeq = previous.length > 0 ? previous[previous.length - 1].seq : 0;
        const reset = payload.entries[0].seq <= lastSeq;
        const merged = reset
          ? payload.entries
          : [...previous, ...payload.entries.filter((entry) => entry.seq > lastSeq)];
        return merged.slice(-MAX_WIRE_ENTRIES);
      });
    });
    return () => {
      void pending.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [serverId, fetchWire]);

  // 通知 / stderr
  useEffect(() => {
    const pending = [
      listen<{ server_id: string; seq: number; method: string; params: unknown; at: string }>(
        "mcp://notification",
        (event) => {
          const payload = event.payload;
          if (payload.server_id !== serverId || pausedRef.current) return;
          setNotifications((previous) =>
            [
              ...previous,
              {
                seq: payload.seq,
                at: payload.at,
                text: `${payload.method} ${searchable(payload.params).slice(0, 300)}`,
              },
            ].slice(-MAX_EVENT_LINES),
          );
          if (payload.method.includes("list_changed")) {
            void queryClient.invalidateQueries({
              queryKey: ["capabilities", payload.server_id],
            });
          }
        },
      ),
      listen<{ server_id: string; seq: number; line: string; at: string }>(
        "mcp://stderr",
        (event) => {
          const payload = event.payload;
          if (payload.server_id !== serverId || pausedRef.current) return;
          setStderr((previous) =>
            [...previous, { seq: payload.seq, at: payload.at, text: payload.line }].slice(
              -MAX_EVENT_LINES,
            ),
          );
        },
      ),
    ];
    return () => {
      for (const subscription of pending) {
        void subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [serverId, queryClient]);

  // 跟随最新
  useEffect(() => {
    if (!follow || !open || tab !== "wire") return;
    const element = wireListRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [wire, follow, open, tab]);

  const interactions = useMemo<WireInteraction[]>(
    () => pairWireEntries(wire),
    [wire],
  );

  const methods = useMemo(() => {
    const set = new Set<string>();
    for (const entry of wire) if (entry.method) set.add(entry.method);
    return [...set].sort();
  }, [wire]);

  const filtered = useMemo(() => {
    const needle = keyword.trim().toLowerCase();
    return interactions.filter((item) => {
      const itemMethod = item.request?.method ?? item.entry?.method ?? "";
      if (method && itemMethod !== method) return false;
      if (onlyErrors && !(item.response?.is_error || item.request?.is_error)) return false;
      if (!needle) return true;
      const haystack = [
        itemMethod,
        item.request?.rpc_id ?? "",
        item.response?.rpc_id ?? "",
        item.request?.summary ?? "",
        item.response?.summary ?? "",
        item.entry?.summary ?? "",
        searchable(item.request?.payload),
        searchable(item.response?.payload),
        searchable(item.entry?.payload),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(needle);
    });
  }, [interactions, keyword, method, onlyErrors]);

  const selected = useMemo(
    () => filtered.find((item) => item.key === selectedKey) ?? null,
    [filtered, selectedKey],
  );

  const counts = {
    wire: interactions.length,
    notifications: notifications.length,
    stderr: stderr.length,
  };

  function togglePause() {
    if (paused) {
      setPaused(false);
      void fetchWire();
    } else {
      setPaused(true);
    }
  }

  async function handleClear() {
    if (tab === "wire") {
      try {
        await clearWireLog(serverId);
        setWire([]);
        setSelectedKey(null);
      } catch (error) {
        toast.fromError(error, "清空报文失败");
      }
      return;
    }
    if (tab === "notifications") {
      setNotifications([]);
      return;
    }
    setStderr([]);
  }

  async function handleExport() {
    try {
      const stamp = new Date()
        .toISOString()
        .slice(0, 19)
        .replace(/[^0-9]/g, "");
      const path = await save({
        title: "导出报文",
        defaultPath: `wire-${stamp}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const saved = await exportWireLog(serverId, path);
      toast.success("已导出报文", {
        message: saved,
        action: {
          label: "打开所在文件夹",
          run: () => {
            void revealItemInDir(saved);
          },
        },
      });
    } catch (error) {
      toast.fromError(error, "导出报文失败");
    }
  }

  return (
    <div className="mt-2 rounded-lg border border-slate-200 bg-white">
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 px-3 py-2 text-xs text-slate-500">
        <button
          type="button"
          className="font-medium text-slate-600 hover:underline"
          onClick={() => openPanel(!open)}
        >
          {open ? "▾" : "▸"} 事件面板
        </button>
        <div className="flex items-center gap-1">
          {(
            [
              ["wire", "报文"],
              ["notifications", "通知"],
              ["stderr", "stderr"],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              type="button"
              className={`rounded px-2 py-0.5 ${
                tab === key
                  ? "bg-slate-800 text-white"
                  : "border border-slate-200 hover:bg-slate-50"
              }`}
              onClick={() => selectTab(key)}
            >
              {label}
              <span className="ml-1 text-[10px] opacity-70">{counts[key]}</span>
            </button>
          ))}
        </div>

        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-0.5 hover:bg-slate-50"
          onClick={togglePause}
        >
          {paused ? "继续" : "暂停"}
        </button>
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-0.5 hover:bg-slate-50"
          onClick={() => void handleClear()}
        >
          清空
        </button>
        {tab === "wire" ? (
          <>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-0.5 hover:bg-slate-50"
              onClick={() => void fetchWire()}
            >
              刷新
            </button>
            <label className="flex items-center gap-1 text-[11px] text-slate-500">
              <input
                type="checkbox"
                checked={follow}
                onChange={(event) => setFollow(event.target.checked)}
              />
              跟随最新
            </label>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-0.5 hover:bg-slate-50"
              onClick={() => void handleExport()}
            >
              导出
            </button>
          </>
        ) : null}
      </div>

      {open && tab === "wire" ? (
        <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 px-3 py-1.5 text-[11px] text-slate-500">
          <input
            className="w-52 rounded border border-slate-200 px-2 py-0.5"
            placeholder="搜索方法 / id / 载荷…"
            value={keyword}
            onChange={(event) => setKeyword(event.target.value)}
          />
          <select
            className="rounded border border-slate-200 px-1.5 py-0.5"
            value={method}
            onChange={(event) => setMethod(event.target.value)}
          >
            <option value="">全部方法</option>
            {methods.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1">
            <input
              type="checkbox"
              checked={onlyErrors}
              onChange={(event) => setOnlyErrors(event.target.checked)}
            />
            仅错误
          </label>
          <span className="text-slate-400">
            共 {filtered.length} 条 · 缓冲 {wire.length}/{MAX_WIRE_ENTRIES}
          </span>
          {paused ? <span className="text-amber-600">已暂停（恢复后自动补齐）</span> : null}
        </div>
      ) : null}

      {open ? (
        tab === "wire" ? (
          <div className="flex min-h-0 gap-2 p-2">
            <div
              ref={wireListRef}
              className="max-h-72 w-[46%] shrink-0 overflow-auto rounded border border-slate-100"
            >
              {wireLoading && wire.length === 0 ? (
                <div className="p-3 text-[11px] text-slate-400">加载报文…</div>
              ) : wireError ? (
                <div className="p-3 text-[11px] text-red-500">报文加载失败：{wireError}</div>
              ) : filtered.length === 0 ? (
                <div className="p-3 text-[11px] text-slate-400">
                  {wire.length === 0
                    ? "暂无报文（连接后 initialize / discover / tools.call 等 JSON-RPC 帧将显示在这里）"
                    : "没有符合筛选条件的报文"}
                </div>
              ) : (
                filtered.map((item) => (
                  <WireRow
                    key={item.key}
                    item={item}
                    selected={item.key === selectedKey}
                    onSelect={() => setSelectedKey(item.key)}
                  />
                ))
              )}
            </div>
            <div className="min-w-0 flex-1 overflow-auto">
              {selected ? (
                <div className="space-y-2">
                  {selected.request ? (
                    <EntryDetail title="请求" entry={selected.request} />
                  ) : null}
                  {selected.response ? (
                    <EntryDetail
                      title="响应"
                      entry={selected.response}
                      duration={selected.response.duration_ms ?? selected.request?.duration_ms ?? null}
                    />
                  ) : null}
                  {selected.entry ? <EntryDetail title="事件" entry={selected.entry} /> : null}
                  {selected.request && !selected.response ? (
                    <div className="text-[10px] text-amber-600">未收到对应响应（可能仍在途或已取消）。</div>
                  ) : null}
                </div>
              ) : (
                <div className="p-3 text-[11px] text-slate-400">
                  选择左侧一条报文查看请求 / 响应详情。
                </div>
              )}
            </div>
          </div>
        ) : tab === "notifications" ? (
          <div className="max-h-64 overflow-y-auto p-2 font-mono text-[11px] leading-5">
            {notifications.length === 0 ? (
              <div className="text-slate-400">暂无通知（服务端通知将显示在这里）</div>
            ) : (
              notifications.slice(-200).map((line) => (
                <div key={`ntf-${line.seq}`} className="flex gap-2 text-slate-700">
                  <span className="shrink-0 text-slate-300">{formatClock(line.at)}</span>
                  <span className="break-all">{line.text}</span>
                </div>
              ))
            )}
          </div>
        ) : (
          <div className="max-h-64 overflow-y-auto p-2 font-mono text-[11px] leading-5">
            {stderr.length === 0 ? (
              <div className="text-slate-400">暂无 stderr 输出（stdio 服务器子进程日志将显示在这里）</div>
            ) : (
              stderr.slice(-200).map((line) => (
                <div key={`err-${line.seq}`} className="flex gap-2 text-slate-500">
                  <span className="shrink-0 text-slate-300">{formatClock(line.at)}</span>
                  <span className="break-all">{line.text}</span>
                </div>
              ))
            )}
          </div>
        )
      ) : null}
    </div>
  );
}

function statusOf(item: WireInteraction): { text: string; className: string } {
  if (item.entry) {
    return item.entry.kind === "system"
      ? { text: "系统", className: "text-slate-400" }
      : { text: "通知", className: "text-slate-400" };
  }
  if (item.response) {
    return item.response.is_error
      ? { text: "错误", className: "text-red-600" }
      : { text: "成功", className: "text-emerald-600" };
  }
  return { text: "在途", className: "text-amber-600" };
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
  const start = item.request ?? item.response ?? item.entry;
  if (!start) return null;
  const status = statusOf(item);
  const request = item.request;
  const response = item.response;
  const direction = request
    ? request.direction === "client_to_server"
      ? "→"
      : "←"
    : start.direction === "client_to_server"
      ? "→"
      : start.direction === "system"
        ? "•"
        : "←";
  const size = (request?.bytes ?? 0) + (response?.bytes ?? 0) || start.bytes;
  const duration = response?.duration_ms ?? request?.duration_ms ?? null;
  const id = request?.rpc_id ?? response?.rpc_id;
  const label =
    request?.method ?? start.method ?? (start.kind === "system" ? start.summary : "message");

  return (
    <button
      type="button"
      className={`flex w-full items-center gap-1.5 border-b border-slate-50 px-2 py-1 text-left font-mono text-[10px] last:border-b-0 ${
        selected ? "bg-slate-100" : "hover:bg-slate-50"
      }`}
      onClick={onSelect}
    >
      <span className="w-[70px] shrink-0 text-slate-400">{formatClock(start.at)}</span>
      <span
        className={`w-3 shrink-0 ${
          direction === "→" ? "text-blue-500" : direction === "←" ? "text-emerald-600" : "text-slate-400"
        }`}
      >
        {direction}
      </span>
      <span className="min-w-0 flex-1 truncate text-slate-700" title={label}>
        {label}
      </span>
      {id ? <span className="w-12 shrink-0 truncate text-slate-400">#{id}</span> : null}
      <span className={`w-9 shrink-0 ${status.className}`}>{status.text}</span>
      <span className="w-12 shrink-0 text-right text-slate-400">{formatDuration(duration)}</span>
      <span className="w-12 shrink-0 text-right text-slate-400">{formatBytes(size)}</span>
    </button>
  );
}

function EntryDetail({
  title,
  entry,
  duration = null,
}: {
  title: string;
  entry: WireEntry;
  duration?: number | null;
}) {
  return (
    <div className="min-w-0">
      <div className="mb-1 flex flex-wrap items-center gap-2 text-[10px] text-slate-500">
        <span className="font-medium text-slate-600">{title}</span>
        <span className="font-mono">{entry.method ?? entry.kind}</span>
        {entry.rpc_id ? <span className="font-mono">#{entry.rpc_id}</span> : null}
        {duration !== null ? <span>耗时 {formatDuration(duration)}</span> : null}
        <span className="ml-auto">{formatBytes(entry.bytes)}</span>
      </div>
      <MessageViewer
        key={entry.seq}
        value={entry.payload}
        defaultTab="tree"
        tabs={["tree", "raw"]}
        contentClassName="max-h-64 overflow-auto"
      />
    </div>
  );
}
