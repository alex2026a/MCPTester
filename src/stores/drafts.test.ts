import { beforeEach, describe, expect, it } from "vitest";
import { draftKey, useDraftStore } from "./drafts";

beforeEach(() => {
  useDraftStore.getState().clearAll();
});

describe("drafts", () => {
  it("save/get 返回副本，空对象删除 key", () => {
    const key = draftKey("srv-1", "tools_call", "echo");
    useDraftStore.getState().save(key, { text: "hello" });
    expect(useDraftStore.getState().get(key)).toEqual({ text: "hello" });

    const copy = useDraftStore.getState().get(key);
    if (copy) copy.text = "changed";
    expect(useDraftStore.getState().get(key)).toEqual({ text: "hello" });

    useDraftStore.getState().save(key, {});
    expect(useDraftStore.getState().get(key)).toBeNull();
  });

  it("clear/clearAll 清理草稿", () => {
    const first = draftKey("srv-1", "tools_call", "echo");
    const second = draftKey("srv-1", "prompts_get", "greet");
    useDraftStore.getState().save(first, { text: "a" });
    useDraftStore.getState().save(second, { who: "b" });

    useDraftStore.getState().clear(first);
    expect(useDraftStore.getState().get(first)).toBeNull();
    expect(useDraftStore.getState().get(second)).toEqual({ who: "b" });

    useDraftStore.getState().clearAll();
    expect(useDraftStore.getState().get(second)).toBeNull();
  });
});
