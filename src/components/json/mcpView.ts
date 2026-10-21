/**
 * MCP 结果解析（v0.1.3）
 *
 * 把 `tools/call` 的 content 块、`resources/read` 的 contents、`prompts/get` 的 messages
 * 归一化为可渲染块（文本 / 图片 / 音频 / 资源 / 链接 / 结构化 JSON）。
 */

interface BlockBase {
  role?: string;
}

export type McpBlock =
  | ({ kind: "text"; text: string } & BlockBase)
  | ({ kind: "image"; mimeType: string; data: string } & BlockBase)
  | ({ kind: "audio"; mimeType: string; data: string } & BlockBase)
  | ({
      kind: "resource";
      uri: string;
      mimeType?: string;
      name?: string;
      text?: string;
      blob?: string;
    } & BlockBase)
  | ({
      kind: "link";
      uri: string;
      name?: string;
      description?: string;
      mimeType?: string;
    } & BlockBase)
  | ({ kind: "json"; label: string; value: unknown } & BlockBase);

export interface McpView {
  blocks: McpBlock[];
  isError: boolean;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function resourceBlock(value: unknown): McpBlock | null {
  const record = asRecord(value);
  if (!record) return null;
  return {
    kind: "resource",
    uri: asString(record.uri) ?? "",
    mimeType: asString(record.mimeType) ?? asString(record.mime_type),
    name: asString(record.name),
    text: asString(record.text),
    blob: asString(record.blob),
  };
}

function blockFromContent(value: unknown): McpBlock | null {
  const block = asRecord(value);
  if (!block) return null;
  const type = asString(block.type) ?? "";
  switch (type) {
    case "text": {
      const text = asString(block.text);
      return text === undefined ? null : { kind: "text", text };
    }
    case "image":
    case "audio": {
      const data = asString(block.data);
      if (!data) return null;
      return {
        kind: type,
        mimeType:
          asString(block.mimeType) ??
          asString(block.mime_type) ??
          (type === "image" ? "image/png" : "audio/wav"),
        data,
      };
    }
    case "resource": {
      return resourceBlock(asRecord(block.resource) ?? block);
    }
    case "resource_link":
    case "link": {
      return {
        kind: "link",
        uri: asString(block.uri) ?? "",
        name: asString(block.name),
        description: asString(block.description),
        mimeType: asString(block.mimeType) ?? asString(block.mime_type),
      };
    }
    default:
      return { kind: "json", label: type || "content", value };
  }
}

function blocksFromMessage(value: unknown): McpBlock[] {
  const message = asRecord(value);
  if (!message) return [];
  const role = asString(message.role);
  const content = message.content;
  const blocks = Array.isArray(content)
    ? content.map(blockFromContent)
    : [blockFromContent(content)];
  return blocks
    .filter((block): block is McpBlock => Boolean(block))
    .map((block) => ({ ...block, role }));
}

/** 构造语义渲染视图；无法识别时返回 null（回退到结构/原文页签） */
export function buildMcpView(method: string | undefined, result: unknown): McpView | null {
  if (!method) return null;
  const record = asRecord(result);
  if (!record) return null;

  let blocks: McpBlock[];
  let isError = false;

  if (method === "tools_call") {
    const content = Array.isArray(record.content) ? record.content : [];
    blocks = content.map(blockFromContent).filter((block): block is McpBlock => Boolean(block));
    const structured = record.structuredContent ?? record.structured_content;
    if (structured !== undefined && structured !== null) {
      blocks.push({ kind: "json", label: "structuredContent", value: structured });
    }
    isError = record.isError === true || record.is_error === true;
    if (blocks.length === 0 && !isError) return null;
  } else if (method === "resources_read") {
    if (!Array.isArray(record.contents)) return null;
    blocks = record.contents
      .map(resourceBlock)
      .filter((block): block is McpBlock => Boolean(block));
    if (blocks.length === 0) return null;
  } else if (method === "prompts_get") {
    if (!Array.isArray(record.messages)) return null;
    blocks = record.messages.flatMap(blocksFromMessage);
    if (blocks.length === 0) return null;
  } else {
    return null;
  }

  return { blocks, isError };
}
