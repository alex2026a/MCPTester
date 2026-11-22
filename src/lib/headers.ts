/**
 * 请求头批量粘贴解析（v0.6.1）
 *
 * 支持 `Name: value`、`Name=value`、cURL `-H` / `--header`。
 * 只解析常见形式，不承诺覆盖所有 shell 转义；Header 名按 RFC 9110 token 校验。
 */

import type { KeyValue } from "../types/models";
import { isSensitiveKey, looksLikeSecret } from "./sensitive";

export interface ParsedHeader {
  key: string;
  value: string;
  secret: boolean;
}

export interface ParseHeaderResult {
  items: ParsedHeader[];
  errors: string[];
  warnings: string[];
  /** 空行、注释、无法识别的行数 */
  ignored: number;
}

export interface MergeHeaderResult {
  headers: KeyValue[];
  added: number;
  updated: number;
}

export const COMMON_HEADER_NAMES = [
  "Authorization",
  "X-Api-Key",
  "X-Request-Id",
  "X-Correlation-Id",
  "X-Tenant-Id",
  "X-Trace-Id",
  "Accept",
  "Content-Type",
  "User-Agent",
];

const HEADER_NAME_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

const RESERVED_HEADERS = new Set([
  "host",
  "content-length",
  "connection",
  "transfer-encoding",
  "upgrade",
]);

/** RFC 9110 token 校验 */
export function isValidHeaderName(name: string): boolean {
  return HEADER_NAME_RE.test(name.trim());
}

function isReservedHeader(name: string): boolean {
  return RESERVED_HEADERS.has(name.trim().toLowerCase());
}

function unquote(value: string): string {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

/** 从一行 cURL 中提取 -H/--header 参数 */
function extractCurlHeaderArgs(line: string): string[] {
  const results: string[] = [];
  const pattern = /(?:^|\s)(?:-H|--header)\s+("([^"]*)"|'([^']*)'|(\S+))/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(line)) !== null) {
    results.push(match[2] ?? match[3] ?? match[4] ?? "");
  }
  return results;
}

/** 解析单行 header；返回 null 表示忽略 */
function parseSingleHeader(raw: string): {
  key: string;
  value: string;
} | null {
  let line = raw.trim();
  if (!line || line.startsWith("#")) return null;

  // 单行 -H / --header
  if (/^(?:-H|--header)\s+/i.test(line)) {
    line = line.replace(/^(?:-H|--header)\s+/i, "");
    line = unquote(line);
  }

  const colon = line.indexOf(":");
  const equals = line.indexOf("=");
  let key: string;
  let value: string;
  if (colon > 0 && (equals < 0 || colon < equals)) {
    key = line.slice(0, colon);
    value = line.slice(colon + 1);
  } else if (equals > 0) {
    key = line.slice(0, equals);
    value = line.slice(equals + 1);
  } else {
    return null;
  }
  return { key: key.trim(), value: value.trim() };
}

export function parseHeaderLines(text: string): ParseHeaderResult {
  const result: ParseHeaderResult = {
    items: [],
    errors: [],
    warnings: [],
    ignored: 0,
  };
  const trimmedText = text.replace(/\r\n/g, "\n").trim();
  if (!trimmedText) return result;

  const byKey = new Map<string, number>();

  const add = (raw: string) => {
    const parsed = parseSingleHeader(raw);
    if (!parsed) {
      result.ignored += 1;
      return;
    }
    if (!parsed.key) {
      result.errors.push(`请求头名称不能为空：${raw.trim()}`);
      return;
    }
    if (!isValidHeaderName(parsed.key)) {
      result.errors.push(`请求头名称非法：${parsed.key}`);
      return;
    }
    if (isReservedHeader(parsed.key)) {
      result.warnings.push(
        `请求头「${parsed.key}」可能由 HTTP 客户端管理，发送结果可能被覆盖`,
      );
    }
    const item: ParsedHeader = {
      key: parsed.key,
      value: parsed.value,
      secret: isSensitiveKey(parsed.key) || looksLikeSecret(parsed.value),
    };
    const lower = parsed.key.toLowerCase();
    const existing = byKey.get(lower);
    if (existing !== undefined) {
      result.items[existing] = item;
      result.warnings.push(`请求头「${parsed.key}」重复，已保留最后一条`);
      return;
    }
    byKey.set(lower, result.items.length);
    result.items.push(item);
  };

  for (const line of trimmedText.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      result.ignored += 1;
      continue;
    }
    if (/^curl(?:\s|$)/i.test(trimmed)) {
      const args = extractCurlHeaderArgs(trimmed);
      if (args.length === 0) {
        result.warnings.push(`未在 cURL 行中找到 -H/--header：${trimmed}`);
        result.ignored += 1;
        continue;
      }
      for (const arg of args) add(arg);
      continue;
    }
    add(trimmed);
  }

  return result;
}

/** 合并解析结果到现有请求头；大小写不敏感，保持已有顺序 */
export function mergeHeaders(
  existing: KeyValue[],
  parsed: ParsedHeader[],
): MergeHeaderResult {
  const headers = existing.map((item) => ({ ...item }));
  const indexByKey = new Map<string, number>();
  headers.forEach((item, index) => {
    indexByKey.set(item.key.trim().toLowerCase(), index);
  });

  let added = 0;
  let updated = 0;
  for (const item of parsed) {
    const lower = item.key.trim().toLowerCase();
    const index = indexByKey.get(lower);
    if (index === undefined) {
      indexByKey.set(lower, headers.length);
      headers.push({ key: item.key, value: item.value, secret: item.secret });
      added += 1;
      continue;
    }
    headers[index] = {
      ...headers[index],
      key: headers[index].key,
      value: item.value,
      secret: item.secret,
    };
    updated += 1;
  }
  return { headers, added, updated };
}
