/**
 * 结果文本摘要（v0.5.0）
 *
 * 只保留可读文本、资源文本、链接、structuredContent；图片/音频 base64 用占位符，
 * 避免把几 MB 二进制复制到剪贴板。
 */

import { buildMcpView, type McpBlock } from "../components/json/mcpView";
import { formatBytes } from "../components/json/format";
import { redactSensitive } from "./sensitive";
import { stringifyJson } from "./exchange";
import type { CallRecord } from "../types/models";

function base64Bytes(data: string): number {
  return Math.max(0, Math.floor((data.length * 3) / 4));
}

function rolePrefix(block: McpBlock): string {
  return block.role ? `[${block.role}] ` : "";
}

function renderBlock(block: McpBlock): string | null {
  switch (block.kind) {
    case "text":
      return block.text.trim() ? `${rolePrefix(block)}${block.text}` : null;
    case "image":
    case "audio":
      return `[${block.kind} ${block.mimeType}, ${formatBytes(base64Bytes(block.data))}, base64 omitted]`;
    case "resource": {
      const meta = [block.uri, block.mimeType].filter(Boolean).join(" · ");
      if (block.text !== undefined) {
        return `[resource ${meta}]${rolePrefix(block)}\n${block.text}`;
      }
      if (block.blob !== undefined) {
        return `[resource ${meta}, ${formatBytes(base64Bytes(block.blob))}, base64 omitted]`;
      }
      return `[resource ${meta}]`;
    }
    case "link":
      return `[link] ${block.uri}${block.name ? `（${block.name}）` : ""}`;
    case "json":
      return `[${block.label}]\n${stringifyJson(block.value)}`;
    default:
      return null;
  }
}

/** 生成可复制的文本摘要；无内容返回 null */
export function buildTextDigest(
  record: CallRecord,
  fullResult?: unknown,
  redact = false,
): string | null {
  const result = fullResult !== undefined ? fullResult : record.result;
  if (result === null || result === undefined) return null;

  const value = redact ? redactSensitive(result).value : result;
  const view = buildMcpView(record.method, value);
  if (!view) return stringifyJson(value);

  const blocks = view.blocks
    .map(renderBlock)
    .filter((block): block is string => Boolean(block));
  return blocks.length > 0 ? blocks.join("\n\n") : null;
}
