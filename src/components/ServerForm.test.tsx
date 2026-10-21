import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ServerForm } from "./ServerForm";
import type { ServerConfig, ServerDraft } from "../types/models";

const initial: ServerConfig = {
  id: "s1",
  name: "demo",
  transport: "streamable_http",
  stdio: null,
  http: {
    url: "http://127.0.0.1:8000/mcp",
    headers: [
      { key: "Authorization", value: "Beare****w0aO", secret: true },
      { key: "X-Trace", value: "abc", secret: false },
    ],
    timeout_secs: null,
    allow_insecure_tls: false,
  },
  protocol_version_pin: null,
  enabled: true,
  tags: [],
  created_at: "2026-10-19T00:00:00Z",
  updated_at: "2026-10-19T00:00:00Z",
  last_connected_at: null,
};

const MASKED_PLACEHOLDER = "已配置（留空=不修改；输入即替换）";

describe("ServerForm 敏感值掩码防护（v0.1.1/v0.1.2）", () => {
  it("掩码值不填入输入框（占位符提示已配置），提交时按 keep", async () => {
    const onSubmit = vi.fn();
    render(<ServerForm initial={initial} onSubmit={onSubmit} onCancel={() => {}} />);

    // v0.1.2：掩码改为占位符显示，输入框为空
    const secretInput = screen.getByPlaceholderText(MASKED_PLACEHOLDER);
    expect(secretInput).toHaveValue("");

    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    const draft = onSubmit.mock.calls[0][0] as ServerDraft;
    const auth = draft.secrets.find((secret) => secret.key === "Authorization");
    expect(auth?.action).toBe("keep");
    expect(auth?.value).toBeNull();
    // 非敏感项仍按 replace 提交
    const trace = draft.secrets.find((secret) => secret.key === "X-Trace");
    expect(trace?.action).toBe("replace");
    expect(trace?.value).toBe("abc");
  });

  it("输入新明文 token 时按 replace 提交（掩码不被写回）", async () => {
    const onSubmit = vi.fn();
    render(<ServerForm initial={initial} onSubmit={onSubmit} onCancel={() => {}} />);

    const secretInput = screen.getByPlaceholderText(MASKED_PLACEHOLDER);
    await userEvent.type(secretInput, "Bearer new-token");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    const draft = onSubmit.mock.calls[0][0] as ServerDraft;
    const auth = draft.secrets.find((secret) => secret.key === "Authorization");
    expect(auth?.action).toBe("replace");
    expect(auth?.value).toBe("Bearer new-token");
  });

  it("本地地址缺端口时显示提示（v0.1.2）", () => {
    const withoutPort: ServerConfig = {
      ...initial,
      http: { ...initial.http!, url: "http://127.0.0.1/mcp", headers: [] },
    };
    render(<ServerForm initial={withoutPort} onSubmit={() => {}} onCancel={() => {}} />);
    expect(screen.getByText(/未指定端口/)).toBeInTheDocument();
  });
});
