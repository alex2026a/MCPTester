import type { CapabilitySnapshot } from "../types/models";

export type InterfaceCategory = "tools" | "resources" | "templates" | "prompts";
export type SearchScope = "name" | "all";
export type ContentLabel = "描述" | "参数" | "字段";

export interface ContentSegment {
  label: ContentLabel;
  text: string;
  lowerText: string;
}

export interface InterfaceEntry {
  key: string;
  title: string;
  subtitle: string;
  /** 名称/标识索引：接口名、资源名与 URI */
  nameText: string;
  /** 内容索引：描述、Prompt 参数、Tool Schema 字段名与说明 */
  content: ContentSegment[];
}

export interface InterfaceSearchHit {
  key: string;
  title: string;
  subtitle: string;
  matchedIn: "name" | "content";
  contentLabel?: ContentLabel;
  snippet?: string;
}

export interface InterfaceSearchResult {
  nameHits: InterfaceSearchHit[];
  contentHits: InterfaceSearchHit[];
  total: number;
}

/** Schema 递归深度上限，避免异常结构拖慢索引构建 */
const MAX_SCHEMA_DEPTH = 32;
/** 单个 Schema 最多收集的字段分段数 */
const MAX_SCHEMA_SEGMENTS = 500;
/** 不参与内容搜索的 Schema 关键字（类型、枚举、约束值等） */
const NON_CONTENT_SCHEMA_KEYS = new Set([
  "type",
  "enum",
  "const",
  "default",
  "examples",
  "example",
  "required",
  "format",
  "pattern",
  "minimum",
  "maximum",
  "exclusiveMinimum",
  "exclusiveMaximum",
  "minLength",
  "maxLength",
  "minItems",
  "maxItems",
  "uniqueItems",
  "multipleOf",
  "$schema",
  "$id",
  "$ref",
  "$comment",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function segment(label: ContentLabel, text: string): ContentSegment {
  return { label, text, lowerText: text.toLowerCase() };
}

function descriptionSegments(description: string | null): ContentSegment[] {
  return description ? [segment("描述", description)] : [];
}

function collectSchemaSegments(
  node: unknown,
  depth: number,
  out: ContentSegment[],
  seen: Set<string>,
): void {
  if (depth > MAX_SCHEMA_DEPTH || out.length >= MAX_SCHEMA_SEGMENTS) return;

  if (Array.isArray(node)) {
    for (const item of node) {
      collectSchemaSegments(item, depth + 1, out, seen);
    }
    return;
  }
  if (!isRecord(node)) return;

  const properties = node.properties;
  if (isRecord(properties)) {
    for (const [name, value] of Object.entries(properties)) {
      const description =
        isRecord(value) && typeof value.description === "string"
          ? value.description
          : null;
      const text = description ? `${name} — ${description}` : name;
      if (!seen.has(text)) {
        seen.add(text);
        out.push(segment("字段", text));
      }
      collectSchemaSegments(value, depth + 1, out, seen);
    }
  }

  for (const [key, value] of Object.entries(node)) {
    if (key === "properties" || NON_CONTENT_SCHEMA_KEYS.has(key)) continue;
    collectSchemaSegments(value, depth + 1, out, seen);
  }
}

function schemaSegments(schema: unknown): ContentSegment[] {
  const segments: ContentSegment[] = [];
  collectSchemaSegments(schema, 0, segments, new Set());
  return segments;
}

export function buildInterfaceEntries(
  snapshot: CapabilitySnapshot,
  category: InterfaceCategory,
): InterfaceEntry[] {
  switch (category) {
    case "tools":
      return snapshot.tools.map((tool) => ({
        key: tool.name,
        title: tool.name,
        subtitle: tool.description ?? "",
        nameText: tool.name.toLowerCase(),
        content: [
          ...descriptionSegments(tool.description),
          ...schemaSegments(tool.input_schema),
        ],
      }));
    case "resources":
      return snapshot.resources.map((resource) => ({
        key: resource.uri,
        title: resource.name,
        subtitle: resource.uri,
        nameText: `${resource.name}\n${resource.uri}`.toLowerCase(),
        content: descriptionSegments(resource.description),
      }));
    case "templates":
      return snapshot.resource_templates.map((template) => ({
        key: template.uri_template,
        title: template.name,
        subtitle: template.uri_template,
        nameText: `${template.name}\n${template.uri_template}`.toLowerCase(),
        content: descriptionSegments(template.description),
      }));
    case "prompts":
      return snapshot.prompts.map((prompt) => ({
        key: prompt.name,
        title: prompt.name,
        subtitle: prompt.description ?? "",
        nameText: prompt.name.toLowerCase(),
        content: [
          ...descriptionSegments(prompt.description),
          ...prompt.arguments.map((argument) =>
            segment(
              "参数",
              argument.description
                ? `${argument.name} — ${argument.description}`
                : argument.name,
            ),
          ),
        ],
      }));
  }
}

/** 截取命中位置附近的上下文，合并换行并加省略号 */
function snippetFor(text: string, keyword: string): string {
  const index = text.toLowerCase().indexOf(keyword);
  if (index < 0) return text;
  const start = Math.max(0, index - 32);
  const end = Math.min(text.length, index + keyword.length + 32);
  const body = text.slice(start, end).replace(/\s+/g, " ").trim();
  return `${start > 0 ? "…" : ""}${body}${end < text.length ? "…" : ""}`;
}

export function searchInterfaceEntries(
  entries: InterfaceEntry[],
  keyword: string,
  scope: SearchScope,
): InterfaceSearchResult {
  const needle = keyword.trim().toLowerCase();
  if (!needle) {
    const nameHits = entries.map((entry) => ({
      key: entry.key,
      title: entry.title,
      subtitle: entry.subtitle,
      matchedIn: "name" as const,
    }));
    return { nameHits, contentHits: [], total: nameHits.length };
  }

  const nameHits: InterfaceSearchHit[] = [];
  const contentHits: InterfaceSearchHit[] = [];

  for (const entry of entries) {
    const base = {
      key: entry.key,
      title: entry.title,
      subtitle: entry.subtitle,
    };
    if (entry.nameText.includes(needle)) {
      nameHits.push({ ...base, matchedIn: "name" });
      continue;
    }
    if (scope !== "all") continue;

    const matched = entry.content.find((item) =>
      item.lowerText.includes(needle),
    );
    if (!matched) continue;
    contentHits.push({
      ...base,
      matchedIn: "content",
      contentLabel: matched.label,
      snippet: snippetFor(matched.text, needle),
    });
  }

  return {
    nameHits,
    contentHits,
    total: nameHits.length + contentHits.length,
  };
}
