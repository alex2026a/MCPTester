/**
 * Streamable HTTP 请求骨架（v0.5.0）
 *
 * 只生成可读骨架，不承诺可直接运行：MCP 需要先建立会话，并携带
 * `Mcp-Session-Id` / 协议版本。头、URL query、body 都会强制脱敏。
 */

import type { ServerConfig } from "../types/models";
import { looksLikeSecret, redactSensitive } from "./sensitive";

const SENSITIVE_HEADERS = new Set([
  "authorization",
  "proxy-authorization",
  "cookie",
  "set-cookie",
  "x-api-key",
  "api-key",
  "x-auth-token",
  "x-access-token",
  "mcp-session-id",
]);

const SENSITIVE_QUERY = new Set([
  "token",
  "access_token",
  "refresh_token",
  "id_token",
  "api_key",
  "apikey",
  "key",
  "secret",
  "password",
]);

function isSensitiveHeader(name: string): boolean {
  const key = name.trim().toLowerCase();
  return (
    SENSITIVE_HEADERS.has(key) ||
    key.includes("api-key") ||
    key.includes("api_key") ||
    key.includes("token") ||
    key.includes("secret")
  );
}

function maskUrl(raw: string): string {
  try {
    const url = new URL(raw);
    if (url.username) url.username = "****";
    if (url.password) url.password = "****";
    for (const key of [...url.searchParams.keys()]) {
      if (SENSITIVE_QUERY.has(key.toLowerCase())) {
        url.searchParams.set(key, "****");
      }
    }
    return url.toString();
  } catch {
    return raw;
  }
}

function maskHeaderValue(name: string, value: string | null): string {
  if (value == null || isSensitiveHeader(name) || looksLikeSecret(value)) {
    return "****";
  }
  return value;
}

function shellQuote(text: string): string {
  return `'${text.replace(/'/g, "'\\''")}'`;
}

function isModernVersion(version: string): boolean {
  const match = /^(\d{4})-(\d{2})-\d{2}$/.exec(version);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  return year > 2026 || (year === 2026 && month >= 7);
}

function protocolNote(protocolVersion: string): string[] {
  if (isModernVersion(protocolVersion)) {
    return [
      "# 现代世代：先 server/discover；后续请求携带 Mcp-Session-Id 与 MCP-Protocol-Version",
      "# 敏感头/URL/body 已脱敏；以下仅为请求骨架，不可直接运行",
    ];
  }
  return [
    "# 旧世代：先 initialize；后续请求携带 Mcp-Session-Id",
    "# 敏感头/URL/body 已脱敏；以下仅为请求骨架，不可直接运行",
  ];
}

/** 生成 Streamable HTTP cURL 骨架；非 Streamable HTTP 返回 null */
export function buildHttpSkeleton(
  server: ServerConfig,
  requestText: string,
  protocolVersion: string,
): string | null {
  if (server.transport !== "streamable_http" || !server.http?.url) {
    return null;
  }

  let body = requestText;
  try {
    const redacted = redactSensitive(JSON.parse(requestText) as unknown);
    body = JSON.stringify(redacted.value, null, 2);
  } catch {
    // 非 JSON body 保留原文；调用方已尽量脱敏
  }

  const headers = new Map<string, string>();
  for (const header of server.http.headers ?? []) {
    const name = header.key.trim();
    if (!name) continue;
    headers.set(
      name.toLowerCase(),
      `${name}: ${maskHeaderValue(name, header.value)}`,
    );
  }

  if (server.http.oauth) {
    headers.delete("authorization");
    headers.set("authorization", "Authorization: Bearer ****");
  }
  if (!headers.has("content-type")) {
    headers.set("content-type", "Content-Type: application/json");
  }
  if (!headers.has("accept")) {
    headers.set("accept", "Accept: application/json, text/event-stream");
  }
  headers.delete("mcp-session-id");
  headers.set("mcp-session-id", "Mcp-Session-Id: <session-id>");
  if (isModernVersion(protocolVersion)) {
    headers.set(
      "mcp-protocol-version",
      `MCP-Protocol-Version: ${protocolVersion}`,
    );
  }

  const lines = [
    "# MCP Streamable HTTP 请求骨架（bash / Git Bash；PowerShell 请用 curl.exe）",
    ...protocolNote(protocolVersion),
    `curl --request POST \\`,
    `  --url ${shellQuote(maskUrl(server.http.url))} \\`,
  ];
  for (const header of headers.values()) {
    lines.push(`  --header ${shellQuote(header)} \\`);
  }
  lines.push(`  --data-raw ${shellQuote(body)}`);
  return lines.join("\n");
}
