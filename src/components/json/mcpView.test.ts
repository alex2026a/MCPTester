import { describe, expect, it } from "vitest";
import { buildMcpView } from "./mcpView";

describe("buildMcpView", () => {
  it("解析 tools/call 的内容块与 structuredContent", () => {
    const view = buildMcpView("tools_call", {
      content: [
        { type: "text", text: "hello" },
        { type: "image", data: "AAAA", mimeType: "image/png" },
      ],
      structuredContent: { ok: true },
      isError: false,
    });
    expect(view).not.toBeNull();
    expect(view?.blocks.map((block) => block.kind)).toEqual(["text", "image", "json"]);
    expect(view?.isError).toBe(false);
  });

  it("isError 标记（含无内容时仍返回视图）", () => {
    const view = buildMcpView("tools_call", { content: [], isError: true });
    expect(view?.isError).toBe(true);
    expect(view?.blocks).toHaveLength(0);
  });

  it("无内容且非错误时返回 null（回退结构视图）", () => {
    expect(buildMcpView("tools_call", { content: [] })).toBeNull();
    expect(buildMcpView("tools_call", null)).toBeNull();
    expect(buildMcpView(undefined, {})).toBeNull();
  });

  it("解析 resources/read 的 contents（文本与二进制）", () => {
    const view = buildMcpView("resources_read", {
      contents: [
        { uri: "file://a.txt", mimeType: "text/plain", text: "abc" },
        { uri: "file://b.bin", mimeType: "application/octet-stream", blob: "AAAA" },
      ],
    });
    expect(view?.blocks).toHaveLength(2);
    expect(view?.blocks[0]).toMatchObject({ kind: "resource", text: "abc" });
    expect(view?.blocks[1]).toMatchObject({ kind: "resource", blob: "AAAA" });
  });

  it("解析 prompts/get 的消息（带角色）", () => {
    const view = buildMcpView("prompts_get", {
      messages: [
        { role: "user", content: { type: "text", text: "问题" } },
        {
          role: "assistant",
          content: [{ type: "text", text: "回答" }, { type: "resource_link", uri: "https://x" }],
        },
      ],
    });
    expect(view?.blocks.map((block) => block.role)).toEqual(["user", "assistant", "assistant"]);
    expect(view?.blocks[2].kind).toBe("link");
  });

  it("未知内容类型降级为 JSON 块", () => {
    const view = buildMcpView("tools_call", { content: [{ type: "mystery", value: 1 }] });
    expect(view?.blocks[0]).toMatchObject({ kind: "json", label: "mystery" });
  });
});
