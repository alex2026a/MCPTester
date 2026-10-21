/**
 * 统一消息查看器（v0.1.3）
 *
 * 三页签：渲染（MCP 语义）/ 结构（可折叠树）/ 原文（高亮 + 虚拟滚动）。
 * 统一工具栏：搜索（计数 + 上下跳 + 结构视图匹配列表）、展开层级、换行、
 * Markdown、复制全部、全屏、大小与截断提示。
 */

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { formatBytes } from "./format";
import { JsonText } from "./JsonText";
import { JsonTree } from "./JsonTree";
import { McpContentView } from "./McpContentView";
import { buildMcpView } from "./mcpView";
import type { JsonMatch } from "./paths";

export type MessageTab = "rendered" | "tree" | "raw";

/** 原文视图单次渲染上限（完整内容仍可复制 / 打开全文） */
const MAX_RAW_PREVIEW = 2_000_000;

export interface MessageViewerProps {
  /** 结构化值（与 text 二选一；同时存在时 text 优先作为原文） */
  value?: unknown;
  /** 预序列化文本 */
  text?: string;
  /** MCP 方法（tools_call / resources_read / prompts_get），用于语义渲染 */
  method?: string;
  /** 后端记录的结果字节数（优先于本地估算） */
  sizeBytes?: number;
  /** 结果是否被截断入库 */
  truncated?: boolean;
  defaultTab?: MessageTab;
  /** 限制可用页签（如报文详情只要结构/原文） */
  tabs?: MessageTab[];
  showToolbar?: boolean;
  /** 结构 / 原文滚动容器样式 */
  contentClassName?: string;
  /** 截断出口：打开完整结果 */
  onOpenFullResult?: () => void;
  /** 截断出口：在文件夹中显示 */
  onRevealFullResult?: () => void;
}

function pickTab(available: MessageTab[], preferred?: MessageTab): MessageTab {
  if (preferred && available.includes(preferred)) return preferred;
  return available[0] ?? "raw";
}

export function MessageViewer({
  value,
  text,
  method,
  sizeBytes,
  truncated = false,
  defaultTab,
  tabs,
  showToolbar = true,
  contentClassName = "max-h-80 overflow-auto",
  onOpenFullResult,
  onRevealFullResult,
}: MessageViewerProps) {
  const rawText = useMemo(() => {
    if (typeof text === "string") return text;
    try {
      return JSON.stringify(value, null, 2) ?? "null";
    } catch {
      return String(value);
    }
  }, [value, text]);

  const parsed = useMemo((): { ok: boolean; value: unknown } => {
    if (value !== undefined) return { ok: true, value };
    try {
      return { ok: true, value: JSON.parse(rawText) as unknown };
    } catch {
      return { ok: false, value: undefined };
    }
  }, [value, rawText]);

  const mcpView = useMemo(
    () => buildMcpView(method, value !== undefined ? value : parsed.value),
    [method, value, parsed],
  );

  const tabsKey = tabs?.join("|") ?? "";
  const availableTabs = useMemo<MessageTab[]>(() => {
    const all: MessageTab[] = [];
    if (mcpView) all.push("rendered");
    if (parsed.ok) all.push("tree");
    all.push("raw");
    const wanted = tabsKey ? (tabsKey.split("|") as MessageTab[]) : null;
    const allowed = wanted ? all.filter((tab) => wanted.includes(tab)) : all;
    return allowed.length > 0 ? allowed : ["raw"];
  }, [mcpView, parsed.ok, tabsKey]);

  const [preferredTab, setPreferredTab] = useState<MessageTab | undefined>(defaultTab);
  const tab: MessageTab =
    preferredTab && availableTabs.includes(preferredTab)
      ? preferredTab
      : pickTab(availableTabs, defaultTab);

  const [search, setSearch] = useState("");
  const [activeMatch, setActiveMatch] = useState(0);
  const [matchCount, setMatchCount] = useState(0);
  const [treeMatches, setTreeMatches] = useState<JsonMatch[]>([]);
  const [matchListOpen, setMatchListOpen] = useState(false);
  const [markdown, setMarkdown] = useState(true);
  const [wrap, setWrap] = useState(false);
  const [expandDepth, setExpandDepth] = useState<number | "all">(2);
  const [copiedAll, setCopiedAll] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);

  const handleMatchCount = useCallback((count: number) => setMatchCount(count), []);
  const handleTreeMatches = useCallback((matches: JsonMatch[]) => setTreeMatches(matches), []);

  useEffect(() => {
    if (!fullscreen) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFullscreen(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [fullscreen]);

  function changeTab(next: MessageTab) {
    setPreferredTab(next);
    setActiveMatch(0);
    setMatchCount(0);
    setTreeMatches([]);
    setMatchListOpen(false);
  }

  function changeSearch(next: string) {
    setSearch(next);
    setActiveMatch(0);
    setMatchCount(0);
    setMatchListOpen(Boolean(next.trim()) && tab === "tree");
  }

  function stepMatch(delta: number) {
    if (matchCount <= 0) return;
    setActiveMatch((current) => (((current + delta) % matchCount) + matchCount) % matchCount);
  }

  async function copyAll() {
    try {
      await navigator.clipboard?.writeText(rawText);
      setCopiedAll(true);
      window.setTimeout(() => setCopiedAll(false), 1200);
    } catch {
      // 剪贴板不可用，忽略
    }
  }

  const byteSize = useMemo(() => {
    if (typeof sizeBytes === "number") return sizeBytes;
    try {
      return new Blob([rawText]).size;
    } catch {
      return rawText.length;
    }
  }, [rawText, sizeBytes]);

  const rawShown = rawText.length > MAX_RAW_PREVIEW ? rawText.slice(0, MAX_RAW_PREVIEW) : rawText;
  const rawTruncated = rawText.length > MAX_RAW_PREVIEW;

  const toolbar = showToolbar ? (
    <div className="flex flex-wrap items-center gap-1.5 border-b border-slate-100 px-2 py-1.5 text-xs">
      <div className="flex items-center gap-1">
        {availableTabs.map((item) => (
          <button
            key={item}
            type="button"
            className={`rounded px-1.5 py-0.5 ${
              tab === item
                ? "bg-slate-800 text-white"
                : "border border-slate-200 text-slate-600 hover:bg-slate-50"
            }`}
            onClick={() => changeTab(item)}
          >
            {item === "rendered" ? "渲染" : item === "tree" ? "结构" : "原文"}
          </button>
        ))}
      </div>

      <div className="relative flex items-center gap-1">
        <input
          className="w-40 rounded border border-slate-200 px-1.5 py-0.5 text-xs"
          placeholder="搜索…"
          value={search}
          onChange={(event) => changeSearch(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              stepMatch(1);
              setMatchListOpen(false);
            } else if (event.key === "Escape") {
              setMatchListOpen(false);
            }
          }}
        />
        {search.trim() ? (
          <span className="min-w-10 text-[10px] text-slate-400">
            {matchCount > 0 ? `${(activeMatch % matchCount) + 1}/${matchCount}` : "0 处"}
          </span>
        ) : null}
        {tab === "tree" && search.trim() && treeMatches.length > 0 ? (
          <button
            type="button"
            className="rounded border border-slate-200 px-1 text-[10px] text-slate-500 hover:bg-slate-50"
            title="匹配列表"
            onClick={() => setMatchListOpen((open) => !open)}
          >
            ▾
          </button>
        ) : null}
        {search.trim() ? (
          <>
            <button
              type="button"
              className="rounded border border-slate-200 px-1 text-[10px] text-slate-500 hover:bg-slate-50 disabled:opacity-40"
              disabled={matchCount <= 0}
              title="上一处"
              onClick={() => stepMatch(-1)}
            >
              ‹
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-1 text-[10px] text-slate-500 hover:bg-slate-50 disabled:opacity-40"
              disabled={matchCount <= 0}
              title="下一处"
              onClick={() => stepMatch(1)}
            >
              ›
            </button>
          </>
        ) : null}

        {matchListOpen && tab === "tree" && treeMatches.length > 0 ? (
          <div className="absolute left-0 top-full z-20 mt-1 max-h-56 w-96 overflow-auto rounded border border-slate-200 bg-white shadow-lg">
            {treeMatches.map((match, index) => (
              <button
                key={`${match.pathText}-${index}`}
                type="button"
                className={`block w-full border-b border-slate-50 px-2 py-1 text-left last:border-b-0 ${
                  index === activeMatch ? "bg-amber-50" : "hover:bg-slate-50"
                }`}
                onClick={() => {
                  setActiveMatch(index);
                  setMatchListOpen(false);
                }}
              >
                <div className="truncate font-mono text-[10px] text-slate-500">{match.pathText}</div>
                <div className="truncate text-[11px] text-slate-700">
                  <span className="mr-1 rounded bg-slate-100 px-1 text-[9px] text-slate-400">
                    {match.kind === "key" ? "键" : "值"}
                  </span>
                  {match.preview}
                </div>
              </button>
            ))}
          </div>
        ) : null}
      </div>

      {tab === "tree" ? (
        <select
          className="rounded border border-slate-200 px-1 py-0.5 text-[10px] text-slate-600"
          value={String(expandDepth)}
          onChange={(event) =>
            setExpandDepth(event.target.value === "all" ? "all" : Number(event.target.value))
          }
          title="展开层级"
        >
          <option value="1">展开 1 层</option>
          <option value="2">展开 2 层</option>
          <option value="3">展开 3 层</option>
          <option value="all">全部展开</option>
        </select>
      ) : null}

      {tab === "raw" ? (
        <button
          type="button"
          className={`rounded border px-1.5 py-0.5 text-[10px] ${
            wrap
              ? "border-slate-700 bg-slate-700 text-white"
              : "border-slate-200 text-slate-600 hover:bg-slate-50"
          }`}
          onClick={() => setWrap((current) => !current)}
          title="自动换行"
        >
          换行
        </button>
      ) : null}

      {tab === "rendered" ? (
        <label className="flex items-center gap-1 text-[10px] text-slate-500">
          <input
            type="checkbox"
            checked={markdown}
            onChange={(event) => setMarkdown(event.target.checked)}
          />
          Markdown
        </label>
      ) : null}

      <span className="ml-auto flex items-center gap-1.5 text-[10px] text-slate-400">
        {truncated ? (
          <span className="rounded bg-amber-100 px-1 py-0.5 text-amber-700">已截断</span>
        ) : null}
        <span>{formatBytes(byteSize)}</span>
      </span>

      {truncated && onOpenFullResult ? (
        <button
          type="button"
          className="rounded border border-amber-200 px-1.5 py-0.5 text-[10px] text-amber-700 hover:bg-amber-50"
          onClick={onOpenFullResult}
        >
          打开全文
        </button>
      ) : null}
      {truncated && onRevealFullResult ? (
        <button
          type="button"
          className="rounded border border-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 hover:bg-slate-50"
          onClick={onRevealFullResult}
        >
          所在文件夹
        </button>
      ) : null}

      <button
        type="button"
        className="rounded border border-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 hover:bg-slate-50"
        onClick={() => void copyAll()}
      >
        {copiedAll ? "已复制" : "复制全部"}
      </button>
      <button
        type="button"
        className="rounded border border-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 hover:bg-slate-50"
        onClick={() => setFullscreen((current) => !current)}
      >
        {fullscreen ? "退出全屏" : "全屏"}
      </button>
    </div>
  ) : null;

  let body: ReactNode;
  if (tab === "rendered" && mcpView) {
    body = (
      <div className={`${fullscreen ? "max-h-[calc(100vh-150px)]" : contentClassName} p-2`}>
        <McpContentView view={mcpView} markdown={markdown} />
      </div>
    );
  } else if (tab === "tree" && parsed.ok) {
    body = (
      <JsonTree
        value={parsed.value}
        search={search}
        activeMatch={activeMatch}
        onMatchCountChange={handleMatchCount}
        onMatchesChange={handleTreeMatches}
        expandDepth={expandDepth}
        className={fullscreen ? "max-h-[calc(100vh-150px)]" : contentClassName}
      />
    );
  } else {
    body = (
      <div className="flex min-h-0 flex-col">
        {rawTruncated ? (
          <div className="border-b border-amber-100 bg-amber-50 px-2 py-1 text-[10px] text-amber-700">
            原文超过 2MB，仅显示前 2MB（复制全部仍为完整内容）。
          </div>
        ) : null}
        <JsonText
          text={rawShown}
          search={search}
          activeMatch={activeMatch}
          onMatchCountChange={handleMatchCount}
          wrap={wrap}
          className={fullscreen ? "max-h-[calc(100vh-170px)]" : contentClassName}
        />
      </div>
    );
  }

  const card = (
    <div
      className={
        fullscreen
          ? "flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-slate-200 bg-white text-xs"
          : "rounded border border-slate-200 bg-white text-xs"
      }
    >
      {toolbar}
      {body}
    </div>
  );

  if (!fullscreen) return card;

  return (
    <div
      className="fixed inset-0 z-50 flex flex-col bg-slate-900/40 p-4"
      role="dialog"
      aria-modal="true"
    >
      {card}
    </div>
  );
}
