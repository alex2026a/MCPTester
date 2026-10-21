import { describe, expect, it } from "vitest";
import {
  extractTemplateVariables,
  matchUriTemplate,
  replayArguments,
  replayTab,
} from "./replay";

describe("replay helpers", () => {
  it("方法映射到调用台页签", () => {
    expect(replayTab("tools_call")).toBe("tools");
    expect(replayTab("resources_read")).toBe("resources");
    expect(replayTab("prompts_get")).toBe("prompts");
    expect(replayTab("probe")).toBeNull();
  });

  it("从历史参数中提取 arguments", () => {
    expect(
      replayArguments({ name: "echo", arguments: { text: "hi" } }),
    ).toEqual({
      text: "hi",
    });
    expect(replayArguments({ uri: "file:///a.txt" })).toEqual({});
    expect(replayArguments(null)).toEqual({});
    expect(replayArguments({ name: "echo", arguments: null })).toEqual({});
  });

  it("提取 URI 模板变量", () => {
    expect(extractTemplateVariables("file:///{path}")).toEqual(["path"]);
    expect(extractTemplateVariables("users://{userId}/posts/{postId}")).toEqual(
      ["userId", "postId"],
    );
    expect(extractTemplateVariables("file:///fixed")).toEqual([]);
  });

  it("用 URI 模板匹配具体 URI", () => {
    expect(matchUriTemplate("file:///{path}", "file:///a/b.txt")).toEqual({
      path: "a/b.txt",
    });
    expect(
      matchUriTemplate("users://{userId}/profile", "users://42/profile"),
    ).toEqual({
      userId: "42",
    });
    expect(matchUriTemplate("users://{userId}", "other://42")).toBeNull();
    expect(matchUriTemplate("file:///fixed", "file:///fixed")).toBeNull();
  });
});
