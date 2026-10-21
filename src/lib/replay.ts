import type { CallMethod } from "../types/models";

/** 重放可定位的页签（resources_read 之后还可能细化为“资源模板”） */
export type ReplayTab = "tools" | "resources" | "prompts";

export const CALL_METHOD_LABEL: Record<CallMethod, string> = {
  tools_call: "tools/call",
  resources_read: "resources/read",
  prompts_get: "prompts/get",
  probe: "probe",
};

/** 历史方法 → 接口页页签；probe 无对应调用台 */
export function replayTab(method: CallMethod): ReplayTab | null {
  switch (method) {
    case "tools_call":
      return "tools";
    case "resources_read":
      return "resources";
    case "prompts_get":
      return "prompts";
    default:
      return null;
  }
}

/** 从历史 params 中取出调用参数（tools/prompts 记录为 { name, arguments }） */
export function replayArguments(params: unknown): Record<string, unknown> {
  if (!params || typeof params !== "object" || Array.isArray(params)) return {};
  const raw = (params as { arguments?: unknown }).arguments;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return raw as Record<string, unknown>;
}

/** 提取 URI 模板变量名（{var}） */
export function extractTemplateVariables(template: string): string[] {
  return [...template.matchAll(/\{([^}]+)\}/g)].map((match) => match[1]);
}

/** 用 URI 模板匹配具体 URI，返回变量值；不匹配返回 null */
export function matchUriTemplate(
  template: string,
  uri: string,
): Record<string, string> | null {
  const names = extractTemplateVariables(template);
  if (names.length === 0) return null;

  const escaped = template.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = escaped.replace(/\\\{([^}]+)\\\}/g, "(.+?)");

  let match: RegExpExecArray | null;
  try {
    match = new RegExp(`^${pattern}$`).exec(uri);
  } catch {
    return null;
  }
  if (!match) return null;

  const values: Record<string, string> = {};
  names.forEach((name, index) => {
    const raw = match?.[index + 1] ?? "";
    try {
      values[name] = decodeURIComponent(raw);
    } catch {
      values[name] = raw;
    }
  });
  return values;
}
