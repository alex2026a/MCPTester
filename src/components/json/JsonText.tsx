/**
 * JSON 原文视图（v0.1.3）
 *
 * - 语法高亮（轻量 tokenizer，不引第三方 highlighter）；
 * - 搜索高亮 + 第 n/m 匹配 + 上下跳；
 * - 行号 + 固定行高虚拟滚动（大报文不卡）；
 * - 默认不换行 + 横向滚动，可切换换行。
 */

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

const LINE_HEIGHT = 20;
const OVERSCAN = 12;
const MAX_MATCHES = 5000;
const MAX_WRAP_LINES = 4000;

interface TextMatch {
  line: number;
  start: number;
  end: number;
  index: number;
}

type TokenKind = "key" | "string" | "number" | "boolean" | "null" | "punct" | "plain";

interface Token {
  start: number;
  text: string;
  kind: TokenKind;
}

const TOKEN_CLASS: Record<TokenKind, string> = {
  key: "text-sky-700",
  string: "text-emerald-700",
  number: "text-blue-700",
  boolean: "text-purple-700",
  null: "text-slate-400",
  punct: "text-slate-400",
  plain: "text-slate-700",
};

/** 单行 JSON 分词（字符串 / 键名 / 数字 / 布尔 / null / 标点） */
function tokenize(line: string): Token[] {
  const tokens: Token[] = [];
  let index = 0;
  let plainStart = 0;

  const flushPlain = (end: number) => {
    if (end > plainStart) {
      tokens.push({ start: plainStart, text: line.slice(plainStart, end), kind: "plain" });
    }
  };

  while (index < line.length) {
    const char = line[index];

    if (char === '"') {
      flushPlain(index);
      const start = index;
      index += 1;
      while (index < line.length) {
        if (line[index] === "\\") {
          index += 2;
          continue;
        }
        if (line[index] === '"') {
          index += 1;
          break;
        }
        index += 1;
      }
      let lookahead = index;
      while (lookahead < line.length && (line[lookahead] === " " || line[lookahead] === "\t")) {
        lookahead += 1;
      }
      tokens.push({
        start,
        text: line.slice(start, index),
        kind: line[lookahead] === ":" ? "key" : "string",
      });
      plainStart = index;
      continue;
    }

    if (char === "-" || (char >= "0" && char <= "9")) {
      const match = /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(line.slice(index));
      if (match) {
        flushPlain(index);
        tokens.push({ start: index, text: match[0], kind: "number" });
        index += match[0].length;
        plainStart = index;
        continue;
      }
    }

    if (line.startsWith("true", index) || line.startsWith("false", index)) {
      const word = line.startsWith("true", index) ? "true" : "false";
      flushPlain(index);
      tokens.push({ start: index, text: word, kind: "boolean" });
      index += word.length;
      plainStart = index;
      continue;
    }

    if (line.startsWith("null", index)) {
      flushPlain(index);
      tokens.push({ start: index, text: "null", kind: "null" });
      index += 4;
      plainStart = index;
      continue;
    }

    const start = index;
    while (index < line.length && /[{}[\],:\s]/.test(line[index])) index += 1;
    if (index === start) {
      // 非 JSON 字符（如纯文本内容）：并入普通片段
      index += 1;
      continue;
    }
    flushPlain(start);
    tokens.push({ start, text: line.slice(start, index), kind: "punct" });
    plainStart = index;
  }

  flushPlain(line.length);
  return tokens;
}

/** 在 token 文本内切开搜索命中区间 */
function renderToken(
  text: string,
  offset: number,
  lineMatches: TextMatch[],
  activeIndex: number,
): ReactNode {
  if (lineMatches.length === 0) return text;
  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const match of lineMatches) {
    const start = Math.max(0, match.start - offset);
    const end = Math.min(text.length, match.end - offset);
    if (end <= 0 || start >= text.length || end <= start) continue;
    if (start > cursor) nodes.push(text.slice(cursor, start));
    nodes.push(
      <mark
        key={`m-${match.index}`}
        className={
          match.index === activeIndex
            ? "rounded bg-amber-400 px-0.5 text-slate-900"
            : "rounded bg-amber-200 px-0.5 text-slate-900"
        }
      >
        {text.slice(start, end)}
      </mark>,
    );
    cursor = end;
  }
  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

export interface JsonTextProps {
  text: string;
  search?: string;
  activeMatch?: number;
  onMatchCountChange?: (count: number) => void;
  wrap?: boolean;
  className?: string;
}

export function JsonText({
  text,
  search = "",
  activeMatch = 0,
  onMatchCountChange,
  wrap = false,
  className = "",
}: JsonTextProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(320);

  const lines = useMemo(() => text.split("\n"), [text]);
  const wrapEnabled = wrap && lines.length <= MAX_WRAP_LINES;

  const matches = useMemo<TextMatch[]>(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return [];
    const found: TextMatch[] = [];
    for (let line = 0; line < lines.length; line += 1) {
      const haystack = lines[line].toLowerCase();
      let from = 0;
      for (;;) {
        const at = haystack.indexOf(needle, from);
        if (at < 0) break;
        found.push({ line, start: at, end: at + needle.length, index: found.length });
        if (found.length >= MAX_MATCHES) return found;
        from = at + Math.max(1, needle.length);
      }
    }
    return found;
  }, [lines, search]);

  const matchesByLine = useMemo(() => {
    const map = new Map<number, TextMatch[]>();
    for (const match of matches) {
      const bucket = map.get(match.line);
      if (bucket) bucket.push(match);
      else map.set(match.line, [match]);
    }
    return map;
  }, [matches]);

  useEffect(() => {
    onMatchCountChange?.(matches.length);
  }, [matches.length, onMatchCountChange]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const update = () => setViewportHeight(element.clientHeight || 320);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const activeIndex =
    matches.length === 0 ? -1 : ((activeMatch % matches.length) + matches.length) % matches.length;

  // 跳转到当前匹配
  useEffect(() => {
    const element = scrollRef.current;
    if (!element || activeIndex < 0) return;
    const match = matches[activeIndex];
    if (!match) return;
    if (wrapEnabled) {
      const row = element.querySelector<HTMLElement>(`[data-line="${match.line}"]`);
      if (row) {
        const containerRect = element.getBoundingClientRect();
        const rowRect = row.getBoundingClientRect();
        element.scrollTop +=
          rowRect.top - containerRect.top - element.clientHeight / 2 + rowRect.height / 2;
        return;
      }
    }
    element.scrollTop = Math.max(0, match.line * LINE_HEIGHT - element.clientHeight / 2);
  }, [activeIndex, matches, wrapEnabled]);

  const digits = Math.max(2, String(lines.length).length);
  const numberWidth = digits * 8 + 12;
  const start = wrapEnabled
    ? 0
    : Math.max(0, Math.floor(scrollTop / LINE_HEIGHT) - OVERSCAN);
  const end = wrapEnabled
    ? lines.length
    : Math.min(lines.length, Math.ceil((scrollTop + viewportHeight) / LINE_HEIGHT) + OVERSCAN);

  const rows: ReactNode[] = [];
  for (let index = start; index < end; index += 1) {
    const lineMatches = matchesByLine.get(index) ?? [];
    const tokens = tokenize(lines[index]);
    rows.push(
      <div
        key={index}
        data-line={index}
        className={`group flex ${wrapEnabled ? "" : "absolute left-0"}`}
        style={
          wrapEnabled
            ? undefined
            : { top: index * LINE_HEIGHT, height: LINE_HEIGHT, width: "max-content", minWidth: "100%" }
        }
      >
        <span
          className="sticky left-0 z-10 shrink-0 select-none bg-white pr-3 text-right text-slate-300 group-hover:bg-slate-50"
          style={{ width: numberWidth }}
        >
          {index + 1}
        </span>
        <code
          className={`block text-slate-700 ${
            wrapEnabled ? "whitespace-pre-wrap break-all" : "whitespace-pre"
          }`}
        >
          {tokens.map((token, tokenIndex) => (
            <span key={tokenIndex} className={TOKEN_CLASS[token.kind]}>
              {renderToken(token.text, token.start, lineMatches, activeIndex)}
            </span>
          ))}
        </code>
      </div>,
    );
  }

  return (
    <div
      ref={scrollRef}
      className={`overflow-auto font-mono text-[11px] leading-5 ${className}`}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div
        className="relative"
        style={wrapEnabled ? undefined : { height: lines.length * LINE_HEIGHT }}
      >
        {rows}
      </div>
      {wrap && !wrapEnabled ? (
        <div className="sticky bottom-0 bg-amber-50 px-2 py-1 text-[10px] text-amber-700">
          内容超过 {MAX_WRAP_LINES} 行，已自动关闭换行以保证滚动流畅。
        </div>
      ) : null}
    </div>
  );
}
