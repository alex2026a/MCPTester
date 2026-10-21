import { describe, expect, it } from "vitest";
import { errorTitle, toErrorInfo } from "./errors";

describe("toErrorInfo", () => {
  it("归一化结构化连接错误（connect_server 失败载荷）", () => {
    const info = toErrorInfo({
      kind: "unauthorized",
      message: '认证失败（HTTP 401）：Bearer error="invalid_token"',
      raw: 'Bearer error="invalid_token"',
    });
    expect(info.kind).toBe("unauthorized");
    expect(info.title).toBe("认证失败");
    expect(info.message).toContain("认证失败");
    expect(info.hint).toContain("Authorization");
    expect(info.raw).toContain("invalid_token");
  });

  it("由连接状态 McpError 字段拼装可读消息", () => {
    const info = toErrorInfo({
      kind: "unauthorized",
      status: 401,
      detail: 'Bearer error="invalid_token"',
    });
    expect(info.message).toContain("401");
    expect(info.message).toContain("invalid_token");
  });

  it("协议版本不受支持时透传服务器支持列表", () => {
    const info = toErrorInfo({
      kind: "unsupported_protocol_version",
      supported: ["2025-11-25", "2025-06-18"],
    });
    expect(info.title).toBe("协议版本不受支持");
    expect(info.message).toContain("2025-11-25");
    expect(info.message).toContain("2025-06-18");
  });

  it("普通字符串错误回退为「操作失败」", () => {
    const info = toErrorInfo("服务器名称不能为空");
    expect(info.kind).toBeNull();
    expect(info.title).toBe("操作失败");
    expect(info.message).toBe("服务器名称不能为空");
  });

  it("凭据掩码遗留给出定向标题与建议（v0.1.2）", () => {
    const info = toErrorInfo({
      kind: "credential_mask",
      message: "请求头「Authorization」当前为掩码（由旧版本编辑保存遗留），无法用于连接。",
    });
    expect(info.title).toBe("凭据需要重新填写");
    expect(info.hint).toContain("清空对应敏感值输入框");
  });
});

describe("errorTitle", () => {
  it("映射已知类别为人话标题", () => {
    expect(errorTitle("handshake_failed")).toBe("握手失败");
    expect(errorTitle("spawn_failed")).toBe("启动进程失败");
  });

  it("未知类别回退带原样 kind", () => {
    expect(errorTitle("weird_kind")).toBe("错误（weird_kind）");
    expect(errorTitle(null)).toBe("错误");
  });
});
