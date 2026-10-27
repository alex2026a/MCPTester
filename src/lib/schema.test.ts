import { describe, expect, it } from "vitest";
import { filterKnownTopLevel } from "./schema";

describe("filterKnownTopLevel", () => {
  it("无 properties / null Schema 保留全部且不提示", () => {
    expect(filterKnownTopLevel(null, { a: 1 })).toEqual({
      value: { a: 1 },
      dropped: 0,
      unknown: 0,
      disallowsAdditional: false,
    });
    expect(filterKnownTopLevel({ type: "object" }, { a: 1 })).toEqual({
      value: { a: 1 },
      dropped: 0,
      unknown: 0,
      disallowsAdditional: false,
    });
  });

  it("空 properties + 开放 Schema 保留且不提示", () => {
    expect(
      filterKnownTopLevel(
        { type: "object", properties: {}, additionalProperties: true },
        { a: 1 },
      ),
    ).toEqual({
      value: { a: 1 },
      dropped: 0,
      unknown: 0,
      disallowsAdditional: false,
    });
  });

  it("空 properties + additionalProperties:false 保留但标记风险", () => {
    expect(
      filterKnownTopLevel(
        { type: "object", properties: {}, additionalProperties: false },
        { a: 1 },
      ),
    ).toEqual({
      value: { a: 1 },
      dropped: 0,
      unknown: 1,
      disallowsAdditional: true,
    });
  });

  it("非空 properties + additionalProperties:false 丢弃未知字段", () => {
    expect(
      filterKnownTopLevel(
        {
          type: "object",
          properties: { text: { type: "string" } },
          additionalProperties: false,
        },
        { text: "hi", extra: 1 },
      ),
    ).toEqual({
      value: { text: "hi" },
      dropped: 1,
      unknown: 0,
      disallowsAdditional: false,
    });
  });

  it("非空 properties + 开放 Schema 保留并统计未声明字段", () => {
    expect(
      filterKnownTopLevel(
        { type: "object", properties: { text: { type: "string" } } },
        { text: "hi", extra: 1 },
      ),
    ).toEqual({
      value: { text: "hi", extra: 1 },
      dropped: 0,
      unknown: 1,
      disallowsAdditional: false,
    });
  });
});
