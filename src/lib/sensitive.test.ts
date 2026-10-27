import { describe, expect, it } from "vitest";
import { findSensitiveFields, redactSensitive } from "./sensitive";

describe("findSensitiveFields", () => {
  it("检测嵌套对象与数组中的敏感键", () => {
    const hits = findSensitiveFields({
      name: "echo",
      arguments: {
        token: "abc",
        nested: { password: "p", items: [{ api_key: "k" }] },
      },
    });
    expect(hits.map((hit) => hit.key)).toEqual([
      "token",
      "password",
      "api_key",
    ]);
    expect(hits.map((hit) => hit.path)).toEqual([
      "$.arguments.token",
      "$.arguments.nested.password",
      "$.arguments.nested.items[0].api_key",
    ]);
  });

  it("检测 Bearer / sk- / JWT 值形态", () => {
    expect(
      findSensitiveFields({
        headers: { Authorization: "Bearer abc123" },
        key: "sk-1234567890",
        jwt: "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signature",
      }).map((hit) => hit.path),
    ).toEqual(["$.headers.Authorization", "$.key", "$.jwt"]);
  });

  it("不误报 max_tokens / token_count / progressToken", () => {
    expect(
      findSensitiveFields({
        max_tokens: 100,
        token_count: 12,
        tokens: ["a", "b"],
        _meta: { progressToken: 7 },
      }),
    ).toEqual([]);
  });

  it("无敏感字段时返回空数组", () => {
    expect(
      findSensitiveFields({ name: "echo", arguments: { text: "hi" } }),
    ).toEqual([]);
    expect(findSensitiveFields(null)).toEqual([]);
  });
});

describe("redactSensitive", () => {
  it("替换敏感键与值形态，保留普通字段", () => {
    const original = {
      token: "abc",
      max_tokens: 100,
      nested: { password: "p", note: "Bearer xyz" },
      items: [{ api_key: "k" }, { text: "hi" }],
    };
    const result = redactSensitive(original);
    expect(result.count).toBe(4);
    expect(result.value).toEqual({
      token: "****",
      max_tokens: 100,
      nested: { password: "****", note: "****" },
      items: [{ api_key: "****" }, { text: "hi" }],
    });
    expect(original.token).toBe("abc");
    expect(original.nested.password).toBe("p");
  });

  it("不脱敏 progressToken", () => {
    const result = redactSensitive({
      params: { _meta: { progressToken: 7 } },
      token: "abc",
    });
    expect(result.value).toEqual({
      params: { _meta: { progressToken: 7 } },
      token: "****",
    });
  });
});
