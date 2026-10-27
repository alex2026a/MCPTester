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

  it("OAuth 模式提交 oauth 配置与 client_secret 三态", async () => {
    const onSubmit = vi.fn();
    const oauthInitial: ServerConfig = {
      ...initial,
      http: {
        url: "https://example.com/mcp",
        headers: [],
        timeout_secs: null,
        allow_insecure_tls: false,
        oauth: {
          scopes: ["openid", "profile"],
          client_id: "client-1",
          client_secret: "****",
          redirect_uri: "http://127.0.0.1:33318/oauth/callback",
          allow_missing_issuer: false,
          authorize_params: [
            { key: "audience", value: "https://api.example.com" },
          ],
          forward_headers_to_auth_server: true,
        },
      },
    };
    render(
      <ServerForm initial={oauthInitial} onSubmit={onSubmit} onCancel={() => {}} />,
    );

    expect(screen.getByLabelText("认证方式")).toHaveValue("oauth");
    expect(screen.getByDisplayValue("openid profile")).toBeInTheDocument();

    await userEvent.type(
      screen.getByPlaceholderText("已配置（留空=不修改；输入即替换）"),
      "new-secret",
    );
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    const draft = onSubmit.mock.calls[0][0] as ServerDraft;
    expect(draft.http?.oauth?.client_id).toBe("client-1");
    expect(draft.http?.oauth?.scopes).toEqual(["openid", "profile"]);
    expect(draft.http?.oauth?.authorize_params).toEqual([
      { key: "audience", value: "https://api.example.com" },
    ]);
    expect(draft.http?.oauth?.forward_headers_to_auth_server).toBe(true);
    const secret = draft.secrets.find((item) => item.kind === "oauth");
    expect(secret?.action).toBe("replace");
    expect(secret?.value).toBe("new-secret");
  });

  it("授权参数格式错误时阻止保存", async () => {
    const onSubmit = vi.fn();
    const oauthInitial: ServerConfig = {
      ...initial,
      http: {
        ...initial.http!,
        url: "https://example.com/mcp",
        oauth: { scopes: [] },
      },
    };
    render(
      <ServerForm initial={oauthInitial} onSubmit={onSubmit} onCancel={() => {}} />,
    );
    const textarea = screen.getByLabelText(/授权请求额外参数/);
    await userEvent.type(textarea, "bad-line");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByText(/缺少 key=value 格式/)).toBeInTheDocument();
  });

  it("切换认证方式会清理 Authorization 头并提交 oauth clear", async () => {
    const onSubmit = vi.fn();
    const oauthInitial: ServerConfig = {
      ...initial,
      http: {
        ...initial.http!,
        oauth: {
          scopes: [],
          client_id: "client-1",
          client_secret: "****",
        },
      },
    };
    render(
      <ServerForm initial={oauthInitial} onSubmit={onSubmit} onCancel={() => {}} />,
    );

    await userEvent.selectOptions(screen.getByLabelText("认证方式"), "none");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    const draft = onSubmit.mock.calls[0][0] as ServerDraft;
    expect(draft.http?.oauth).toBeNull();
    expect(
      draft.http?.headers.some(
        (header) => header.key.toLowerCase() === "authorization",
      ),
    ).toBe(false);
    const secret = draft.secrets.find((item) => item.kind === "oauth");
    expect(secret?.action).toBe("clear");
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
