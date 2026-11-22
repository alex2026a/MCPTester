import { describe, expect, it } from "vitest";
import { supportsHeaderParams, toolHeaderParams } from "./headerParams";

describe("toolHeaderParams", () => {
  it("提取直接顶层 string/integer/boolean 的 x-mcp-header", () => {
    const result = toolHeaderParams({
      type: "object",
      properties: {
        region: { type: "string", "x-mcp-header": "Region" },
        count: { type: "integer", "x-mcp-header": "Count" },
        flag: { type: "boolean", "x-mcp-header": "Flag" },
        query: { type: "string" },
      },
    });
    expect(result).toEqual([
      { argument: "region", header: "Mcp-Param-Region" },
      { argument: "count", header: "Mcp-Param-Count" },
      { argument: "flag", header: "Mcp-Param-Flag" },
    ]);
  });

  it("忽略 number、嵌套、$ref、allOf 与无效 token", () => {
    const result = toolHeaderParams({
      type: "object",
      properties: {
        number: { type: "number", "x-mcp-header": "Number" },
        nested: {
          type: "object",
          properties: { inner: { type: "string", "x-mcp-header": "Inner" } },
        },
        ref: { $ref: "#/$defs/x", "x-mcp-header": "Ref" },
        invalid: { type: "string", "x-mcp-header": "bad:name" },
        empty: { type: "string", "x-mcp-header": "" },
      },
    });
    expect(result).toEqual([]);
  });

  it("header 大小写不敏感去重，保留第一个", () => {
    const result = toolHeaderParams({
      type: "object",
      properties: {
        a: { type: "string", "x-mcp-header": "Region" },
        b: { type: "string", "x-mcp-header": "region" },
      },
    });
    expect(result).toEqual([{ argument: "a", header: "Mcp-Param-Region" }]);
  });
});

describe("supportsHeaderParams", () => {
  it("仅 Streamable HTTP + 2026-07-28 及以后", () => {
    expect(supportsHeaderParams("streamable_http", "2026-07-28")).toBe(true);
    expect(supportsHeaderParams("streamable_http", "2026-08-01")).toBe(true);
    expect(supportsHeaderParams("streamable_http", "2025-11-25")).toBe(false);
    expect(supportsHeaderParams("streamable_http", null)).toBe(false);
    expect(supportsHeaderParams("stdio", "2026-07-28")).toBe(false);
    expect(supportsHeaderParams("sse", "2026-07-28")).toBe(false);
  });
});
