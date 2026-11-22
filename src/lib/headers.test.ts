import { describe, expect, it } from "vitest";
import {
  COMMON_HEADER_NAMES,
  isValidHeaderName,
  mergeHeaders,
  parseHeaderLines,
} from "./headers";
import type { KeyValue } from "../types/models";

describe("parseHeaderLines", () => {
  it("解析 Name: value 与 Name=value，并识别敏感头", () => {
    const result = parseHeaderLines(
      "Authorization: Bearer abc\nX-Api-Key=xyz\nX-Trace: 1",
    );
    expect(result.errors).toEqual([]);
    expect(result.items).toHaveLength(3);
    expect(result.items[0]).toMatchObject({
      key: "Authorization",
      value: "Bearer abc",
      secret: true,
    });
    expect(result.items[1]).toMatchObject({
      key: "X-Api-Key",
      value: "xyz",
      secret: true,
    });
    expect(result.items[2]).toMatchObject({
      key: "X-Trace",
      value: "1",
      secret: false,
    });
  });

  it("解析 cURL 的 -H 与 --header", () => {
    const result = parseHeaderLines(
      "curl -X POST https://example.com -H 'X-Trace: 1' --header \"X-Api-Key: abc\"",
    );
    expect(result.items).toHaveLength(2);
    expect(result.items[0]).toMatchObject({ key: "X-Trace", value: "1" });
    expect(result.items[1]).toMatchObject({
      key: "X-Api-Key",
      value: "abc",
      secret: true,
    });
  });

  it("忽略空行与注释，并计数", () => {
    const result = parseHeaderLines("\n# comment\nX-Trace: 1\n\n");
    expect(result.items).toHaveLength(1);
    expect(result.ignored).toBe(1);
  });

  it("非法 Header 名进入 errors，合法行仍保留", () => {
    const result = parseHeaderLines("Bad Name: x\nX-Trace: 1");
    expect(result.items).toHaveLength(1);
    expect(result.errors[0]).toContain("非法");
  });

  it("重复 key 大小写不敏感，保留最后一条并 warning", () => {
    const result = parseHeaderLines("X-Trace: 1\nx-trace: 2");
    expect(result.items).toHaveLength(1);
    expect(result.items[0].value).toBe("2");
    expect(result.warnings.some((item) => item.includes("重复"))).toBe(true);
  });

  it("保留字段产生 warning", () => {
    const result = parseHeaderLines("Host: example.com");
    expect(result.items).toHaveLength(1);
    expect(result.warnings.some((item) => item.includes("Host"))).toBe(true);
  });

  it("值中包含等号/冒号不会被截断", () => {
    const result = parseHeaderLines("X-Url: https://example.com?a=1");
    expect(result.items[0].value).toBe("https://example.com?a=1");
  });
});

describe("mergeHeaders", () => {
  const existing: KeyValue[] = [
    { key: "Authorization", value: "****", secret: true },
    { key: "X-Trace", value: "old", secret: false },
  ];

  it("同名更新、异名追加并返回计数", () => {
    const result = mergeHeaders(existing, [
      { key: "authorization", value: "Bearer new", secret: true },
      { key: "X-Api-Key", value: "abc", secret: true },
    ]);
    expect(result.added).toBe(1);
    expect(result.updated).toBe(1);
    expect(result.headers).toHaveLength(3);
    expect(result.headers[0]).toMatchObject({
      key: "Authorization",
      value: "Bearer new",
      secret: true,
    });
    expect(result.headers[2]).toMatchObject({
      key: "X-Api-Key",
      value: "abc",
      secret: true,
    });
  });

  it("不修改传入数组", () => {
    mergeHeaders(existing, [{ key: "X-Trace", value: "new", secret: false }]);
    expect(existing[1].value).toBe("old");
  });
});

describe("helpers", () => {
  it("isValidHeaderName 按 RFC 9110 token 校验", () => {
    expect(isValidHeaderName("X-Api-Key")).toBe(true);
    expect(isValidHeaderName("X Trace")).toBe(false);
    expect(isValidHeaderName("")).toBe(false);
  });

  it("常用头包含 Authorization / X-Api-Key", () => {
    expect(COMMON_HEADER_NAMES).toContain("Authorization");
    expect(COMMON_HEADER_NAMES).toContain("X-Api-Key");
  });
});
