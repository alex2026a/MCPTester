/**
 * MCP 2026-07-28 `x-mcp-header` 参数头（v0.6.1）
 *
 * 只做 UI 提示与 CSV 映射；实际发送由 rmcp 负责。
 * 仅 Streamable HTTP + 协议 >= 2026-07-28 + tools/call + 直接顶层
 * string/integer/boolean 生效；`number`、嵌套、`$ref`、`allOf` 不提示。
 */

export interface ToolHeaderParam {
  argument: string;
  /** 实际 HTTP 头名：Mcp-Param-<name> */
  header: string;
}

const HEADER_TOKEN_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

export function supportsHeaderParams(
  transport: string,
  protocolVersion: string | null,
): boolean {
  return (
    transport === "streamable_http" && (protocolVersion ?? "") >= "2026-07-28"
  );
}

export function toolHeaderParams(schema: unknown): ToolHeaderParam[] {
  if (!schema || typeof schema !== "object" || Array.isArray(schema)) {
    return [];
  }
  const properties = (schema as Record<string, unknown>).properties;
  if (
    !properties ||
    typeof properties !== "object" ||
    Array.isArray(properties)
  ) {
    return [];
  }
  const seen = new Set<string>();
  const result: ToolHeaderParam[] = [];
  for (const [argument, raw] of Object.entries(
    properties as Record<string, unknown>,
  )) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const field = raw as Record<string, unknown>;
    if (field.$ref !== undefined) continue;
    const header = field["x-mcp-header"];
    if (typeof header !== "string" || !HEADER_TOKEN_RE.test(header)) continue;
    if (!["string", "integer", "boolean"].includes(String(field.type))) {
      continue;
    }
    const lower = header.toLowerCase();
    if (seen.has(lower)) continue;
    seen.add(lower);
    result.push({ argument, header: `Mcp-Param-${header}` });
  }
  return result;
}
