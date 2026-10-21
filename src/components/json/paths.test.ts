import { describe, expect, it } from "vitest";
import { collectMatches, formatJsonPath, previewValue, valueAtPath } from "./paths";

describe("formatJsonPath", () => {
  it("标识符键用点号，特殊字符键用方括号转义", () => {
    expect(formatJsonPath([])).toBe("$");
    expect(formatJsonPath(["content", 0, "text"])).toBe("$.content[0].text");
    expect(formatJsonPath(["a.b"])).toBe('$["a.b"]');
    expect(formatJsonPath(["中文", 3])).toBe('$["中文"][3]');
    expect(formatJsonPath(["with space"])).toBe('$["with space"]');
    expect(formatJsonPath(['q"uote'])).toBe('$["q\\"uote"]');
  });
});

describe("valueAtPath", () => {
  const root = { content: [{ text: "hi" }], "a.b": 1 };

  it("按路径取值", () => {
    expect(valueAtPath(root, ["content", 0, "text"])).toEqual({ found: true, value: "hi" });
    expect(valueAtPath(root, ["a.b"])).toEqual({ found: true, value: 1 });
  });

  it("越界或不存在时返回 found=false", () => {
    expect(valueAtPath(root, ["content", 9]).found).toBe(false);
    expect(valueAtPath(root, ["missing"]).found).toBe(false);
    expect(valueAtPath(root, ["a.b", "c"]).found).toBe(false);
  });
});

describe("previewValue", () => {
  it("按类型给出摘要", () => {
    expect(previewValue("hello")).toBe('"hello"');
    expect(previewValue(12)).toBe("12");
    expect(previewValue(null)).toBe("null");
    expect(previewValue([1, 2])).toBe("[2 项]");
    expect(previewValue({ a: 1 })).toBe("{1 个键}");
  });
});

describe("collectMatches", () => {
  const value = {
    content: [
      { type: "text", text: "echo: hi" },
      { type: "text", text: "second" },
    ],
    isError: false,
  };

  it("收集键名与值匹配，顺序与渲染一致（先序）", () => {
    const { matches } = collectMatches(value, "text");
    expect(matches.map((match) => match.pathText)).toEqual([
      "$.content[0].type",
      "$.content[0].text",
      "$.content[1].type",
      "$.content[1].text",
    ]);
    expect(matches[0].kind).toBe("value");
    expect(matches[1].kind).toBe("key");
  });

  it("返回祖先容器用于展开", () => {
    const { matches } = collectMatches(value, "hi");
    expect(matches).toHaveLength(1);
    expect(matches[0].kind).toBe("value");
    expect(matches[0].pathText).toBe("$.content[0].text");
    expect(matches[0].ancestors).toContain(value);
    expect(matches[0].ancestors).toContain(value.content);
  });

  it("空查询返回空结果，超出上限标记 truncated", () => {
    expect(collectMatches(value, "  ").matches).toHaveLength(0);
    const many = Array.from({ length: 50 }, (_, index) => ({ index }));
    const limited = collectMatches(many, "index", 10);
    expect(limited.matches).toHaveLength(10);
    expect(limited.truncated).toBe(true);
  });

  it("大小写不敏感", () => {
    expect(collectMatches(value, "ECho").matches).toHaveLength(1);
  });
});
