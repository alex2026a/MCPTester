import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InputRequestDialog } from "./InputRequestDialog";
import type { PendingInput } from "../stores/inputRequests";

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
}));

function item(requests: PendingInput["requests"]): PendingInput {
  return {
    requestId: "r1",
    context: {
      server_id: "s1",
      server_name: "Demo",
      call_id: "c1",
      method: "tools_call",
      target: "echo",
      round: 1,
    },
    requests,
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    minimized: false,
  };
}

const formRequest = {
  kind: "elicitation" as const,
  key: "name",
  mode: "form" as const,
  message: "请输入名称",
  schema: {
    type: "object",
    properties: { name: { type: "string", minLength: 1 } },
    required: ["name"],
  },
};

describe("InputRequestDialog", () => {
  it("表单接受后提交 content", async () => {
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(
      <InputRequestDialog
        item={item([formRequest])}
        visible
        onRespond={onRespond}
        onCancel={vi.fn()}
        onMinimize={vi.fn()}
      />,
    );

    await userEvent.type(screen.getByLabelText("name"), "Ada");
    await userEvent.click(screen.getByRole("button", { name: "提交" }));
    await waitFor(() =>
      expect(onRespond).toHaveBeenCalledWith([
        { key: "name", action: "accept", content: { name: "Ada" } },
      ]),
    );
  });

  it("拒绝不校验表单并提交 decline", async () => {
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(
      <InputRequestDialog
        item={item([formRequest])}
        visible
        onRespond={onRespond}
        onCancel={vi.fn()}
        onMinimize={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "拒绝" }));
    await userEvent.click(screen.getByRole("button", { name: "提交" }));
    await waitFor(() =>
      expect(onRespond).toHaveBeenCalledWith([
        { key: "name", action: "decline" },
      ]),
    );
  });

  it("多条请求一次提交", async () => {
    const onRespond = vi.fn().mockResolvedValue(undefined);
    const city = {
      kind: "elicitation" as const,
      key: "city",
      mode: "form" as const,
      message: "请输入城市",
      schema: {
        type: "object",
        properties: { city: { type: "string" } },
        required: ["city"],
      },
    };
    render(
      <InputRequestDialog
        item={item([formRequest, city])}
        visible
        onRespond={onRespond}
        onCancel={vi.fn()}
        onMinimize={vi.fn()}
      />,
    );

    await userEvent.type(screen.getByLabelText("name"), "Ada");
    await userEvent.type(screen.getByLabelText("city"), "Paris");
    await userEvent.click(
      screen.getByRole("button", { name: "提交全部（2）" }),
    );
    await waitFor(() =>
      expect(onRespond).toHaveBeenCalledWith([
        { key: "name", action: "accept", content: { name: "Ada" } },
        { key: "city", action: "accept", content: { city: "Paris" } },
      ]),
    );
  });

  it("URL 模式可打开链接并接受", async () => {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(
      <InputRequestDialog
        item={item([
          {
            kind: "elicitation",
            key: "auth",
            mode: "url",
            message: "请在浏览器完成",
            url: "https://example.com/auth",
          },
        ])}
        visible
        onRespond={onRespond}
        onCancel={vi.fn()}
        onMinimize={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "打开链接" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.com/auth");
    await userEvent.click(screen.getByRole("button", { name: "提交" }));
    await waitFor(() =>
      expect(onRespond).toHaveBeenCalledWith([
        { key: "auth", action: "accept" },
      ]),
    );
  });

  it("取消与最小化回调", async () => {
    const onCancel = vi.fn().mockResolvedValue(undefined);
    const onMinimize = vi.fn();
    render(
      <InputRequestDialog
        item={item([formRequest])}
        visible
        onRespond={vi.fn()}
        onCancel={onCancel}
        onMinimize={onMinimize}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "取消调用" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
    await userEvent.click(
      screen.getAllByRole("button", { name: "稍后处理" })[0],
    );
    expect(onMinimize).toHaveBeenCalledTimes(1);
  });
});
