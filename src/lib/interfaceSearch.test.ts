import { describe, expect, it } from "vitest";
import {
  buildInterfaceEntries,
  searchInterfaceEntries,
  type InterfaceCategory,
} from "./interfaceSearch";
import type { CapabilitySnapshot } from "../types/models";

const snapshot: CapabilitySnapshot = {
  server_id: "srv-1",
  tools: [
    {
      name: "search_files",
      title: null,
      description: "在项目中搜索文件",
      input_schema: {
        type: "object",
        properties: {
          query: { type: "string", description: "搜索关键词" },
          limit: { type: "integer" },
          filter: {
            type: "object",
            properties: { tag: { type: "string" } },
          },
        },
      },
      output_schema: null,
      annotations: null,
    },
    {
      name: "echo",
      title: null,
      description: "回显文本",
      input_schema: {
        type: "object",
        properties: { text: { type: "string" } },
      },
      output_schema: null,
      annotations: null,
    },
    {
      name: "weather",
      title: null,
      description: "查询天气。search 关键词只应作为内容命中",
      input_schema: null,
      output_schema: null,
      annotations: null,
    },
  ],
  resources: [
    {
      uri: "file:///readme.md",
      name: "README",
      title: null,
      description: "项目说明文档",
      mime_type: "text/markdown",
      size: null,
      annotations: null,
    },
  ],
  resource_templates: [
    {
      uri_template: "file:///{path}",
      name: "workspace-files",
      title: null,
      description: "按路径读取文件",
      mime_type: null,
    },
  ],
  prompts: [
    {
      name: "greet",
      title: null,
      description: "生成问候语\n用于测试 关键词 的上下文",
      arguments: [
        { name: "who", description: "被问候的人", required: true },
        { name: "tone", description: null, required: false },
      ],
    },
  ],
  fetched_at: "2026-11-14T00:00:00Z",
  stale: false,
  warnings: [],
};

function entries(category: InterfaceCategory) {
  return buildInterfaceEntries(snapshot, category);
}

describe("interfaceSearch", () => {
  it("默认名称范围只匹配接口名，不搜索描述", () => {
    const nameResult = searchInterfaceEntries(
      entries("tools"),
      "搜索文件",
      "name",
    );
    expect(nameResult.total).toBe(0);

    const allResult = searchInterfaceEntries(
      entries("tools"),
      "搜索文件",
      "all",
    );
    expect(allResult.nameHits).toHaveLength(0);
    expect(allResult.contentHits.map((hit) => hit.key)).toEqual([
      "search_files",
    ]);
    expect(allResult.contentHits[0].contentLabel).toBe("描述");
    expect(allResult.contentHits[0].snippet).toContain("搜索文件");
  });

  it("名称命中优先分组，且不会重复进入内容分组", () => {
    const result = searchInterfaceEntries(entries("tools"), "search", "all");

    expect(result.nameHits.map((hit) => hit.key)).toEqual(["search_files"]);
    expect(result.contentHits.map((hit) => hit.key)).toEqual(["weather"]);
    expect(result.total).toBe(2);
  });

  it("资源名、URI 与模板 URI 属于名称范围", () => {
    expect(
      searchInterfaceEntries(entries("resources"), "readme", "name").nameHits[0]
        ?.key,
    ).toBe("file:///readme.md");
    expect(
      searchInterfaceEntries(entries("resources"), "file:///", "name")
        .nameHits[0]?.key,
    ).toBe("file:///readme.md");
    expect(
      searchInterfaceEntries(entries("templates"), "workspace", "name")
        .nameHits[0]?.key,
    ).toBe("file:///{path}");
    expect(
      searchInterfaceEntries(entries("templates"), "{path}", "name").nameHits[0]
        ?.key,
    ).toBe("file:///{path}");

    const description = searchInterfaceEntries(
      entries("resources"),
      "项目说明",
      "all",
    );
    expect(description.contentHits[0]?.key).toBe("file:///readme.md");
    expect(description.contentHits[0]?.contentLabel).toBe("描述");
  });

  it("Prompt 参数名与参数说明参与内容搜索", () => {
    expect(
      searchInterfaceEntries(entries("prompts"), "who", "name").total,
    ).toBe(0);

    const byName = searchInterfaceEntries(entries("prompts"), "who", "all");
    expect(byName.contentHits[0]?.key).toBe("greet");
    expect(byName.contentHits[0]?.contentLabel).toBe("参数");
    expect(byName.contentHits[0]?.snippet).toContain("who");

    const byDescription = searchInterfaceEntries(
      entries("prompts"),
      "被问候",
      "all",
    );
    expect(byDescription.contentHits[0]?.key).toBe("greet");
    expect(byDescription.contentHits[0]?.contentLabel).toBe("参数");
  });

  it("Tool Schema 字段名与字段说明参与内容搜索，类型值不参与", () => {
    expect(
      searchInterfaceEntries(entries("tools"), "query", "name").total,
    ).toBe(0);

    const byField = searchInterfaceEntries(entries("tools"), "query", "all");
    expect(byField.contentHits[0]?.key).toBe("search_files");
    expect(byField.contentHits[0]?.contentLabel).toBe("字段");
    expect(byField.contentHits[0]?.snippet).toContain("query");

    const nested = searchInterfaceEntries(entries("tools"), "tag", "all");
    expect(nested.contentHits[0]?.key).toBe("search_files");
    expect(nested.contentHits[0]?.contentLabel).toBe("字段");

    const byDescription = searchInterfaceEntries(
      entries("tools"),
      "关键词",
      "all",
    );
    expect(byDescription.contentHits[0]?.contentLabel).toBe("字段");

    expect(
      searchInterfaceEntries(entries("tools"), "integer", "all").total,
    ).toBe(0);
    expect(
      searchInterfaceEntries(entries("tools"), "string", "all").total,
    ).toBe(0);
  });

  it("空关键词返回全部条目，不产生内容命中", () => {
    for (const category of [
      "tools",
      "resources",
      "templates",
      "prompts",
    ] as InterfaceCategory[]) {
      const source = entries(category);
      const result = searchInterfaceEntries(source, "   ", "all");
      expect(result.nameHits).toHaveLength(source.length);
      expect(result.contentHits).toHaveLength(0);
      expect(result.total).toBe(source.length);
    }
  });

  it("内容片段合并换行并保留命中上下文", () => {
    const result = searchInterfaceEntries(entries("prompts"), "关键词", "all");
    expect(result.contentHits[0]?.snippet).not.toContain("\n");
    expect(result.contentHits[0]?.snippet).toContain("关键词");
  });
});
