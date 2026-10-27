import { describe, expect, it } from "vitest";
import { buildTextDigest } from "./textDigest";
import type { CallRecord } from "../types/models";

function record(partial: Partial<CallRecord> = {}): CallRecord {
  return {
    id: "c1",
    server_id: "s1",
    server_name: "mock",
    method: "tools_call",
    target: "echo",
    params: {},
    result: null,
    error: null,
    status: "ok",
    duration_ms: 1,
    protocol_version: "2026-07-28",
    transport: "stdio",
    truncated: false,
    size_bytes: 0,
    origin: "manual",
    created_at: "2026-10-18T00:00:00Z",
    ...partial,
  };
}

describe("buildTextDigest", () => {
  it("保留文本，图片/音频用占位符", () => {
    const data = "A".repeat(4000);
    const text = buildTextDigest(
      record({
        result: {
          content: [
            { type: "text", text: "hello" },
            { type: "image", mimeType: "image/png", data },
            { type: "audio", mimeType: "audio/wav", data },
          ],
        },
      }),
    );
    expect(text).toContain("hello");
    expect(text).toContain("[image image/png");
    expect(text).toContain("[audio audio/wav");
    expect(text).toContain("base64 omitted");
    expect(text).not.toContain(data);
  });

  it("提示词结果保留 role", () => {
    const text = buildTextDigest(
      record({
        method: "prompts_get",
        result: {
          messages: [
            { role: "user", content: { type: "text", text: "hi" } },
            {
              role: "assistant",
              content: [{ type: "text", text: "hello" }],
            },
          ],
        },
      }),
    );
    expect(text).toContain("[user] hi");
    expect(text).toContain("[assistant] hello");
  });

  it("空结果返回 null；未知结构回退 JSON", () => {
    expect(buildTextDigest(record({ result: null }))).toBeNull();
    const text = buildTextDigest(
      record({ method: "probe", result: { ok: true } }),
    );
    expect(text).toContain('"ok": true');
  });

  it("脱敏选项替换结果中的敏感字段", () => {
    const text = buildTextDigest(
      record({
        result: {
          content: [{ type: "text", text: "hi" }],
          structuredContent: { token: "abc" },
        },
      }),
      undefined,
      true,
    );
    expect(text).toContain('"token": "****"');
  });
});
