/**
 * JSON 路径与搜索匹配（v0.1.3）
 *
 * - JSONPath 生成（键名含特殊字符时使用方括号转义）；
 * - 先序遍历收集"键名 / 原始值"匹配，供结构视图与原文视图共用；
 * - 路径取值（复制值、匹配预览）。
 */

export type JsonPath = (string | number)[];

export interface JsonMatch {
  path: JsonPath;
  pathText: string;
  kind: "key" | "value";
  /** 匹配预览（键名 + 值摘要） */
  preview: string;
  /** 从根到父节点的容器对象（用于展开路径） */
  ancestors: object[];
}

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

function truncate(text: string, max: number): string {
  if (max <= 0 || text.length <= max) return text;
  return `${text.slice(0, Math.max(0, max - 1))}…`;
}

/** 值摘要（列表/匹配预览） */
export function previewValue(value: unknown, max = 80): string {
  if (value === null) return "null";
  switch (typeof value) {
    case "string":
      return truncate(`"${value}"`, max);
    case "number":
    case "boolean":
    case "bigint":
      return String(value);
    case "undefined":
      return "undefined";
    default:
      break;
  }
  if (Array.isArray(value)) return `[${value.length} 项]`;
  if (typeof value === "object") {
    return `{${Object.keys(value as Record<string, unknown>).length} 个键}`;
  }
  return truncate(String(value), max);
}

/** 单个路径段的 JSONPath 表示 */
export function formatPathSegment(segment: string | number): string {
  if (typeof segment === "number") return `[${segment}]`;
  return IDENTIFIER.test(segment) ? `.${segment}` : `[${JSON.stringify(segment)}]`;
}

/** 完整 JSONPath（如 `$.content[0].text`、`$["a.b"][0]`） */
export function formatJsonPath(path: JsonPath): string {
  return `$${path.map(formatPathSegment).join("")}`;
}

/** 按路径取值 */
export function valueAtPath(
  root: unknown,
  path: JsonPath,
): { found: boolean; value?: unknown } {
  let current = root;
  for (const segment of path) {
    if (typeof segment === "number") {
      if (!Array.isArray(current) || segment < 0 || segment >= current.length) {
        return { found: false };
      }
      current = current[segment];
      continue;
    }
    if (
      current === null ||
      typeof current !== "object" ||
      !Object.prototype.hasOwnProperty.call(current, segment)
    ) {
      return { found: false };
    }
    current = (current as Record<string, unknown>)[segment];
  }
  return { found: true, value: current };
}

/**
 * 先序遍历收集匹配（键名 + 原始值），最多 `limit` 条。
 * 返回顺序与结构视图渲染顺序一致，便于"第 n 个匹配"跳转。
 */
export function collectMatches(
  root: unknown,
  query: string,
  limit = 300,
): { matches: JsonMatch[]; truncated: boolean } {
  const needle = query.trim().toLowerCase();
  if (!needle) return { matches: [], truncated: false };

  const matches: JsonMatch[] = [];
  let truncated = false;

  const visit = (value: unknown, path: JsonPath, parents: object[]): boolean => {
    if (matches.length >= limit) {
      truncated = true;
      return false;
    }

    if (Array.isArray(value)) {
      const nextParents = [...parents, value];
      for (let index = 0; index < value.length; index += 1) {
        if (!visit(value[index], [...path, index], nextParents)) return false;
      }
      return true;
    }

    if (value !== null && typeof value === "object") {
      const nextParents = [...parents, value];
      for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
        if (matches.length >= limit) {
          truncated = true;
          return false;
        }
        const childPath = [...path, key];
        if (key.toLowerCase().includes(needle)) {
          matches.push({
            path: childPath,
            pathText: formatJsonPath(childPath),
            kind: "key",
            preview: `${key}: ${previewValue(child)}`,
            ancestors: nextParents,
          });
        }
        if (!visit(child, childPath, nextParents)) return false;
      }
      return true;
    }

    if (previewValue(value, Infinity).toLowerCase().includes(needle)) {
      matches.push({
        path,
        pathText: formatJsonPath(path),
        kind: "value",
        preview: previewValue(value),
        ancestors: parents,
      });
    }
    return true;
  };

  visit(root, [], []);
  return { matches, truncated };
}
