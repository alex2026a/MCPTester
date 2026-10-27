/**
 * JSON 结构视图（v0.1.3，基于 react18-json-view）
 *
 * - 折叠预览 / 按层级展开 / 大容器自动折叠；
 * - 悬浮节点显示 JSONPath，可复制值 / 路径；
 * - 搜索：命中路径强制展开 + 高亮定位（DOM 寻址）。
 */

import JsonView from "react18-json-view";
import "react18-json-view/src/style.css";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  collectMatches,
  formatJsonPath,
  previewValue,
  valueAtPath,
  type JsonMatch,
  type JsonPath,
} from "./paths";
import {
  clearHighlight,
  findPairByPath,
  highlightPair,
  pathFromElement,
  scrollPairIntoView,
} from "./domPaths";

/** 大容器自动折叠阈值（避免深层大数组一次渲染） */
const AUTO_COLLAPSE_ITEMS = 500;
/** 长字符串折叠阈值 */
const COLLAPSE_STRING_AFTER = 400;
/** 匹配上限（与 paths.collectMatches 默认一致） */
const MATCH_LIMIT = 300;
/** 定位重试帧数（库在 effect 中重算折叠状态，需等待渲染完成） */
const LOCATE_RETRY_FRAMES = 5;

export interface JsonTreeProps {
  value: unknown;
  search?: string;
  activeMatch?: number;
  onMatchCountChange?: (count: number) => void;
  onMatchesChange?: (matches: JsonMatch[]) => void;
  expandDepth?: number | "all";
  className?: string;
  /** 在固定高度容器内铺满（Explorer 分栏结果区） */
  fill?: boolean;
}

interface HoverInfo {
  pathText: string;
  preview: string;
  path: JsonPath;
}

export function JsonTree({
  value,
  search = "",
  activeMatch = 0,
  onMatchCountChange,
  onMatchesChange,
  expandDepth = 2,
  className = "",
  fill = false,
}: JsonTreeProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [copied, setCopied] = useState<"value" | "path" | null>(null);

  const { matches } = useMemo(
    () => collectMatches(value, search, MATCH_LIMIT),
    [value, search],
  );

  useEffect(() => {
    onMatchCountChange?.(matches.length);
  }, [matches.length, onMatchCountChange]);

  useEffect(() => {
    onMatchesChange?.(matches);
  }, [matches, onMatchesChange]);

  const activeIndex =
    matches.length === 0 ? -1 : ((activeMatch % matches.length) + matches.length) % matches.length;

  // 命中路径的祖先（含父容器）强制展开；大容器仍自动折叠
  const collapsed = useMemo(() => {
    const forced = new WeakSet<object>();
    const match = activeIndex >= 0 ? matches[activeIndex] : undefined;
    if (match) {
      for (const ancestor of match.ancestors) forced.add(ancestor);
      if (match.path.length > 0) {
        const parent = valueAtPath(value, match.path.slice(0, -1));
        if (parent.found && parent.value && typeof parent.value === "object") {
          forced.add(parent.value as object);
        }
      }
    }
    return ({
      node,
      depth,
      size,
    }: {
      node: Record<string, unknown> | unknown[];
      depth: number;
      size: number;
    }) => {
      if (node !== null && typeof node === "object" && forced.has(node as object)) return false;
      if (size > AUTO_COLLAPSE_ITEMS) return true;
      return expandDepth === "all" ? false : depth > expandDepth;
    };
  }, [matches, activeIndex, value, expandDepth]);

  // 渲染完成后定位并高亮（库重算折叠在 effect 中，故重试若干帧）
  useEffect(() => {
    if (activeIndex < 0) return;
    let frame = 0;
    let attempts = 0;
    const locate = () => {
      const container = containerRef.current;
      const root = container?.querySelector(".json-view");
      const match = matches[activeIndex];
      if (container && root && match) {
        const pair = findPairByPath(root, match.path);
        if (pair) {
          highlightPair(pair, container);
          scrollPairIntoView(container, pair);
          return;
        }
      }
      attempts += 1;
      if (attempts < LOCATE_RETRY_FRAMES) frame = requestAnimationFrame(locate);
    };
    frame = requestAnimationFrame(locate);
    return () => cancelAnimationFrame(frame);
  }, [activeIndex, matches]);

  // 清除命中高亮
  useEffect(() => {
    if (activeIndex >= 0) return;
    clearHighlight(containerRef.current?.querySelector(".json-view"));
  }, [activeIndex]);

  // 树元素保持引用稳定，避免悬浮状态变化导致整树重渲染
  const tree = useMemo(
    () => (
      <JsonView
        src={value}
        theme="default"
        className="mt-json-tree"
        collapsed={collapsed}
        collapseObjectsAfterLength={AUTO_COLLAPSE_ITEMS}
        collapseStringsAfterLength={COLLAPSE_STRING_AFTER}
        displaySize="collapsed"
        enableClipboard
        matchesURL
      />
    ),
    [value, collapsed],
  );

  function handleHover(event: React.MouseEvent<HTMLDivElement>) {
    const container = containerRef.current;
    const root = container?.querySelector(".json-view");
    if (!container || !root) return;
    const pair = (event.target as Element).closest(".json-view--pair");
    if (!pair) {
      setHover(null);
      return;
    }
    const path = pathFromElement(pair, root);
    if (!path) {
      setHover(null);
      return;
    }
    const pathText = formatJsonPath(path);
    if (hover?.pathText === pathText) return;
    const found = valueAtPath(value, path);
    setHover({ path, pathText, preview: found.found ? previewValue(found.value) : "" });
  }

  async function copy(kind: "value" | "path") {
    if (!hover) return;
    let text = hover.pathText;
    if (kind === "value") {
      const found = valueAtPath(value, hover.path);
      if (!found.found) return;
      text =
        typeof found.value === "string"
          ? found.value
          : JSON.stringify(found.value, null, 2) ?? "null";
    }
    try {
      await navigator.clipboard?.writeText(text);
      setCopied(kind);
      window.setTimeout(() => setCopied(null), 1200);
    } catch {
      // 剪贴板不可用，忽略
    }
  }

  return (
    <div className={`relative min-h-0 ${fill ? "h-full" : ""}`}>
      <div
        ref={containerRef}
        className={`overflow-auto font-mono text-[11px] leading-5 ${
          fill ? "h-full" : ""
        } ${className}`}
        onMouseOver={handleHover}
        onMouseLeave={() => setHover(null)}
      >
        {tree}
      </div>
      {hover ? (
        <div className="absolute bottom-1 right-2 flex max-w-[92%] items-center gap-1.5 rounded border border-slate-200 bg-white/95 px-1.5 py-1 text-[10px] shadow-sm">
          <span className="truncate font-mono text-slate-500" title={hover.pathText}>
            {hover.pathText}
          </span>
          <span className="max-w-40 truncate text-slate-400" title={hover.preview}>
            {hover.preview}
          </span>
          <button
            type="button"
            className="shrink-0 rounded border border-slate-200 px-1 py-0.5 text-slate-600 hover:bg-slate-50"
            onClick={() => void copy("value")}
          >
            {copied === "value" ? "已复制" : "复值"}
          </button>
          <button
            type="button"
            className="shrink-0 rounded border border-slate-200 px-1 py-0.5 text-slate-600 hover:bg-slate-50"
            onClick={() => void copy("path")}
          >
            {copied === "path" ? "已复制" : "复径"}
          </button>
        </div>
      ) : null}
    </div>
  );
}
