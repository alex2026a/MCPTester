/**
 * MCP 结果渲染（v0.1.3）
 *
 * 按块渲染文本 / 图片 / 音频 / 资源 / 链接 / 结构化 JSON；
 * 文本块可选 Markdown 渲染（启发式：像 Markdown 才渲染）。
 */

import { useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Markdown } from "./Markdown";
import { formatBytes } from "./format";
import type { McpBlock, McpView } from "./mcpView";

const MAX_TEXT_CHARS = 300_000;
const MAX_INLINE_BASE64 = 2_700_000; // ≈2MB 二进制
const MAX_JSON_CHARS = 120_000;

const MARKDOWN_HINT =
  /(^|\n)\s*(#{1,6}\s|[-*+]\s|\d+\.\s|>\s|```|\|.*\|)|(\*\*[^*\n]+\*\*|`[^`\n]+`|\[[^\]\n]+\]\([^)\n]+\))/;

function decodeBase64Utf8(base64: string): string | null {
  if (typeof atob !== "function") return null;
  try {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return new TextDecoder().decode(bytes);
  } catch {
    return null;
  }
}

function isTextualMime(mimeType: string | undefined): boolean {
  if (!mimeType) return false;
  return (
    mimeType.startsWith("text/") ||
    /json|xml|yaml|javascript|typescript|csv|markdown|x-www-form-urlencoded/i.test(mimeType)
  );
}

function truncateText(text: string): { text: string; truncated: boolean } {
  if (text.length <= MAX_TEXT_CHARS) return { text, truncated: false };
  return { text: text.slice(0, MAX_TEXT_CHARS), truncated: true };
}

function CopyButton({ text, label = "复制" }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="shrink-0 rounded border border-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 hover:bg-slate-50"
      onClick={() => {
        void navigator.clipboard
          ?.writeText(text)
          .then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1200);
          })
          .catch(() => {});
      }}
    >
      {copied ? "已复制" : label}
    </button>
  );
}

function BlockShell({
  title,
  meta,
  children,
  actions,
}: {
  title: string;
  meta?: string;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="rounded border border-slate-200 bg-white">
      <div className="flex items-center gap-2 border-b border-slate-100 px-2 py-1 text-[10px] text-slate-500">
        <span className="font-medium text-slate-600">{title}</span>
        {meta ? <span className="truncate text-slate-400">{meta}</span> : null}
        <span className="ml-auto flex items-center gap-1">{actions}</span>
      </div>
      <div className="p-2 text-xs text-slate-700">{children}</div>
    </div>
  );
}

function TextBlock({
  text,
  role,
  markdown,
}: {
  text: string;
  role?: string;
  markdown: boolean;
}) {
  const shown = truncateText(text);
  const asMarkdown = markdown && MARKDOWN_HINT.test(text);
  return (
    <BlockShell
      title={role ? `text · ${role}` : "text"}
      meta={`${text.length} 字符`}
      actions={<CopyButton text={text} label="复制文本" />}
    >
      {asMarkdown ? (
        <Markdown text={text} />
      ) : (
        <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-5 text-slate-700">
          {shown.text}
        </pre>
      )}
      {shown.truncated ? (
        <div className="mt-1 text-[10px] text-amber-600">
          内容过长，仅显示前 {MAX_TEXT_CHARS.toLocaleString()} 字符（原文页签可查看完整内容）
        </div>
      ) : null}
    </BlockShell>
  );
}

function MediaBlock({
  kind,
  mimeType,
  data,
}: {
  kind: "image" | "audio";
  mimeType: string;
  data: string;
}) {
  const dataUri = `data:${mimeType};base64,${data}`;
  const bytes = Math.max(0, Math.floor((data.length * 3) / 4));
  const inline = data.length <= MAX_INLINE_BASE64;
  return (
    <BlockShell
      title={kind === "image" ? "image" : "audio"}
      meta={`${mimeType} · ${formatBytes(bytes)}`}
      actions={<CopyButton text={dataUri} label="复制 data URI" />}
    >
      {!inline ? (
        <div className="text-[11px] text-slate-500">
          媒体过大（{formatBytes(bytes)}），已取消内嵌预览。
        </div>
      ) : kind === "image" ? (
        <img
          src={dataUri}
          alt="工具返回图片"
          className="max-h-96 max-w-full rounded border border-slate-200 object-contain"
        />
      ) : (
        <audio controls src={dataUri} className="w-full" />
      )}
    </BlockShell>
  );
}

function ResourceBlock({ block }: { block: Extract<McpBlock, { kind: "resource" }> }) {
  const mimeType = block.mimeType;
  const decoded =
    block.text === undefined &&
    block.blob &&
    isTextualMime(mimeType) &&
    block.blob.length <= 2_000_000
      ? decodeBase64Utf8(block.blob)
      : null;
  const text = block.text ?? decoded ?? undefined;
  const shown = text !== undefined ? truncateText(text) : null;

  return (
    <BlockShell
      title={block.name ? `resource · ${block.name}` : "resource"}
      meta={[block.uri, mimeType].filter(Boolean).join(" · ")}
      actions={
        <>
          {block.uri ? <CopyButton text={block.uri} label="复制 URI" /> : null}
          {block.blob ? <CopyButton text={block.blob} label="复制 base64" /> : null}
        </>
      }
    >
      {shown ? (
        <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-5">
          {shown.text}
        </pre>
      ) : block.blob ? (
        <div className="text-[11px] text-slate-500">
          二进制内容 · {formatBytes(Math.floor((block.blob.length * 3) / 4))}（
          {mimeType ?? "未知类型"}）
        </div>
      ) : (
        <div className="text-[11px] text-slate-400">（无内容）</div>
      )}
      {shown?.truncated ? (
        <div className="mt-1 text-[10px] text-amber-600">
          内容过长，仅显示前 {MAX_TEXT_CHARS.toLocaleString()} 字符。
        </div>
      ) : null}
    </BlockShell>
  );
}

function LinkBlock({ block }: { block: Extract<McpBlock, { kind: "link" }> }) {
  const external = /^https?:\/\//i.test(block.uri);
  return (
    <BlockShell
      title={block.name ? `link · ${block.name}` : "link"}
      meta={[block.uri, block.mimeType].filter(Boolean).join(" · ")}
      actions={
        <>
          <CopyButton text={block.uri} label="复制 URI" />
          {external ? (
            <button
              type="button"
              className="shrink-0 rounded border border-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 hover:bg-slate-50"
              onClick={() => void openUrl(block.uri).catch(() => {})}
            >
              打开
            </button>
          ) : null}
        </>
      }
    >
      {block.description ? <div className="break-words">{block.description}</div> : null}
      {!block.description ? (
        <div className="break-all font-mono text-[11px]">{block.uri}</div>
      ) : null}
    </BlockShell>
  );
}

function JsonBlock({ label, value }: { label: string; value: unknown }) {
  const text = (() => {
    try {
      return JSON.stringify(value, null, 2) ?? "null";
    } catch {
      return String(value);
    }
  })();
  const shown = truncateText(text);
  return (
    <BlockShell
      title={label}
      meta={`${text.length} 字符`}
      actions={<CopyButton text={text} label="复制 JSON" />}
    >
      <pre className="max-h-80 overflow-auto whitespace-pre font-mono text-[11px] leading-5">
        {shown.text.slice(0, MAX_JSON_CHARS)}
      </pre>
      {text.length > MAX_JSON_CHARS ? (
        <div className="mt-1 text-[10px] text-amber-600">
          内容较长，仅显示前 12 万字符（结构页签可查看完整内容）。
        </div>
      ) : null}
    </BlockShell>
  );
}

function BlockView({ block, markdown }: { block: McpBlock; markdown: boolean }) {
  switch (block.kind) {
    case "text":
      return <TextBlock text={block.text} role={block.role} markdown={markdown} />;
    case "image":
    case "audio":
      return <MediaBlock kind={block.kind} mimeType={block.mimeType} data={block.data} />;
    case "resource":
      return <ResourceBlock block={block} />;
    case "link":
      return <LinkBlock block={block} />;
    case "json":
      return <JsonBlock label={block.label} value={block.value} />;
    default:
      return null;
  }
}

export type ErrorTone = "error" | "warning" | "neutral";

const ERROR_TONE_CLASS: Record<ErrorTone, string> = {
  error: "border-red-200 bg-red-50 text-red-700",
  warning: "border-amber-200 bg-amber-50 text-amber-700",
  neutral: "border-slate-200 bg-slate-50 text-slate-600",
};

const ERROR_TONE_LABEL: Record<ErrorTone, string> = {
  error: "工具返回错误（isError = true）",
  warning: "工具返回错误（isError = true，按用例配置降级为警告）",
  neutral: "工具返回错误（isError = true，符合用例期望）",
};

export function McpContentView({
  view,
  markdown,
  errorTone = "error",
}: {
  view: McpView;
  markdown: boolean;
  errorTone?: ErrorTone;
}) {
  return (
    <div className="space-y-2">
      {view.isError ? (
        <div
          className={`rounded border px-2 py-1.5 text-xs ${ERROR_TONE_CLASS[errorTone]}`}
        >
          {ERROR_TONE_LABEL[errorTone]}
        </div>
      ) : null}
      {view.blocks.length === 0 ? (
        <div className="text-[11px] text-slate-400">（无内容）</div>
      ) : (
        view.blocks.map((block, index) => (
          <BlockView key={index} block={block} markdown={markdown} />
        ))
      )}
    </div>
  );
}
