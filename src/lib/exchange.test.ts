import { describe, expect, it } from "vitest";
import type { CallRecord } from "../types/models";
import {
  buildPairJson,
  buildPairMarkdown,
  buildRequestEnvelope,
  buildRequestParamsText,
  buildRequestText,
  buildResponseBodyText,
  buildResponseEnvelope,
  buildResponseEnvelopeText,
  responsePayload,
} from "./exchange";

function record(partial: Partial<CallRecord> = {}): CallRecord {
  return {
    id: "c1",
    server_id: "s1",
    server_name: "mock",
    method: "tools_call",
    target: "echo",
    params: { name: "echo", arguments: { text: "hi" } },
    result: { content: [{ type: "text", text: "echo: hi" }] },
    error: null,
    status: "ok",
    duration_ms: 12,
    protocol_version: "2026-07-28",
    transport: "stdio",
    truncated: false,
    size_bytes: 48,
    origin: "manual",
    created_at: "2026-10-18T00:00:00Z",
    ...partial,
  };
}

describe("exchange", () => {
  it("重建 tools/call 请求，id 使用本地记录 ID", () => {
    const request = buildRequestEnvelope(record());
    expect(request).toEqual({
      jsonrpc: "2.0",
      id: "c1",
      method: "tools/call",
      params: { name: "echo", arguments: { text: "hi" } },
    });
    expect(JSON.parse(buildRequestText(record()) ?? "null")).toEqual(request);
    expect(JSON.parse(buildRequestParamsText(record()))).toEqual({
      name: "echo",
      arguments: { text: "hi" },
    });
  });

  it("重建 resources/read 与 prompts/get 请求", () => {
    expect(
      buildRequestEnvelope(
        record({
          method: "resources_read",
          target: "file:///a",
          params: { uri: "file:///a" },
        }),
      ),
    ).toMatchObject({ method: "resources/read", params: { uri: "file:///a" } });

    expect(
      buildRequestEnvelope(
        record({
          method: "prompts_get",
          target: "greet",
          params: { name: "greet", arguments: { name: "Ada" } },
        }),
      ),
    ).toMatchObject({
      method: "prompts/get",
      params: { name: "greet", arguments: { name: "Ada" } },
    });
  });

  it("arguments 为 null 时不把 null 写进请求 params", () => {
    const noArgs = record({ params: { name: "echo", arguments: null } });
    expect(buildRequestEnvelope(noArgs)?.params).toEqual({ name: "echo" });
    expect(JSON.parse(buildRequestParamsText(noArgs))).toEqual({
      name: "echo",
    });
  });

  it("有原始帧时优先使用原始请求/响应", () => {
    const frames = {
      rpc_id: 0,
      request: {
        jsonrpc: "2.0" as const,
        id: 0,
        method: "tools/call",
        params: { name: "echo", arguments: { text: "hi" } },
      },
      response: {
        jsonrpc: "2.0" as const,
        id: 0,
        result: { content: [{ type: "text", text: "echo: hi" }] },
      },
      truncated: false,
    };
    expect(buildRequestEnvelope(record(), frames)).toEqual(frames.request);
    expect(buildResponseEnvelope(record(), undefined, frames)).toEqual(
      frames.response,
    );
    expect(JSON.parse(buildRequestText(record(), frames) ?? "null")).toEqual(
      frames.request,
    );

    const pair = JSON.parse(buildPairJson(record(), undefined, frames));
    expect(pair.meta.frame_source).toEqual({
      request: "original",
      response: "original",
    });
    expect(pair.request.id).toBe(0);
    expect(pair.response.id).toBe(0);
  });

  it("原始帧超限时用 rpc_id 重建并保留 number/string 类型", () => {
    const frames = {
      rpc_id: 7,
      request: null,
      response: null,
      truncated: true,
    };
    expect(buildRequestEnvelope(record(), frames)?.id).toBe(7);
    expect(
      buildResponseEnvelope(
        record({ result: { ok: true }, size_bytes: 12 }),
        undefined,
        frames,
      )?.id,
    ).toBe(7);
    const pair = JSON.parse(buildPairJson(record(), undefined, frames));
    expect(pair.meta.frame_source).toEqual({
      request: "reconstructed",
      response: "reconstructed",
    });
  });

  it("读取完整结果时用完整 body 重建响应，不采用截断的原始响应帧", () => {
    const frames = {
      rpc_id: "abc",
      request: null,
      response: {
        jsonrpc: "2.0" as const,
        id: "abc",
        result: { partial: true },
      },
      truncated: false,
    };
    const full = { content: [{ type: "text", text: "full" }] };
    expect(buildResponseEnvelope(record(), full, frames)).toEqual({
      jsonrpc: "2.0",
      id: "abc",
      result: full,
    });
  });

  it("脱敏选项替换敏感字段但保留 max_tokens", () => {
    const sensitive = record({
      params: {
        name: "echo",
        arguments: { token: "abc", max_tokens: 100 },
      },
    });
    const request = JSON.parse(
      buildRequestText(sensitive, null, true) ?? "null",
    );
    expect(request.params.arguments).toEqual({
      token: "****",
      max_tokens: 100,
    });
    const pair = buildPairMarkdown(sensitive, undefined, null, true);
    expect(pair).toContain('"token": "****"');
    expect(pair).toContain('"max_tokens": 100');
  });

  it("探测记录没有标准 JSON-RPC 请求/响应", () => {
    const probe = record({ method: "probe", target: "probe", params: {} });
    expect(buildRequestEnvelope(probe)).toBeNull();
    expect(buildRequestText(probe)).toBeNull();
    expect(buildResponseEnvelope(probe)).toBeNull();
    expect(buildResponseEnvelopeText(probe)).toBeNull();
  });

  it("复制响应默认是 result body", () => {
    expect(responsePayload(record())).toEqual({
      kind: "result",
      value: { content: [{ type: "text", text: "echo: hi" }] },
    });
    expect(JSON.parse(buildResponseBodyText(record()) ?? "null")).toEqual({
      content: [{ type: "text", text: "echo: hi" }],
    });
  });

  it("isError=true 时响应仍是 result 信封，而不是 JSON-RPC error", () => {
    const isError = record({
      status: "error",
      result: { content: [{ type: "text", text: "boom" }], isError: true },
      error: {
        kind: "rpc_error",
        code: 0,
        message: "工具返回 isError=true",
        data: { content: [{ type: "text", text: "boom" }], isError: true },
      },
    });
    expect(JSON.parse(buildResponseBodyText(isError) ?? "null")).toMatchObject({
      isError: true,
    });
    const envelope = buildResponseEnvelope(isError);
    expect(envelope?.result).toMatchObject({ isError: true });
    expect(envelope?.error).toBeUndefined();

    const pair = JSON.parse(buildPairJson(isError));
    expect(pair.response.result).toMatchObject({ isError: true });
    expect(pair.response.error).toBeUndefined();
    expect(pair.app_error.kind).toBe("rpc_error");
  });

  it("真正的 RPC 错误才重建 error 信封", () => {
    const rpcError = record({
      status: "error",
      result: null,
      size_bytes: 0,
      error: {
        kind: "rpc_error",
        code: -32602,
        message: "参数错误",
        data: { field: "text" },
      },
    });
    expect(JSON.parse(buildResponseBodyText(rpcError) ?? "null")).toEqual({
      kind: "rpc_error",
      code: -32602,
      message: "参数错误",
      data: { field: "text" },
    });
    expect(buildResponseEnvelope(rpcError)).toEqual({
      jsonrpc: "2.0",
      id: "c1",
      error: { code: -32602, message: "参数错误", data: { field: "text" } },
    });
    expect(JSON.parse(buildPairJson(rpcError)).app_error).toBeNull();
  });

  it("应用层错误没有 JSON-RPC 响应", () => {
    const timeout = record({
      status: "error",
      result: null,
      error: { kind: "timeout", message: "调用超时（1s）", timeout_secs: 1 },
    });
    expect(buildResponseEnvelope(timeout)).toBeNull();
    expect(buildResponseEnvelopeText(timeout)).toBeNull();
    expect(JSON.parse(buildResponseBodyText(timeout) ?? "null")).toMatchObject({
      kind: "timeout",
    });
  });

  it("取消记录没有响应 body", () => {
    const canceled = record({
      status: "canceled",
      result: null,
      error: null,
      size_bytes: 0,
    });
    expect(responsePayload(canceled)).toEqual({ kind: "none" });
    expect(buildResponseBodyText(canceled)).toBeNull();
  });

  it("截断结果可传入完整 result 后再生成响应", () => {
    const truncated = record({
      truncated: true,
      result: null,
      size_bytes: 2_000_000,
    });
    const full = { content: [{ type: "text", text: "full" }] };
    expect(buildResponseBodyText(truncated)).toBeNull();
    expect(
      JSON.parse(buildResponseBodyText(truncated, full) ?? "null"),
    ).toEqual(full);
    expect(buildResponseEnvelope(truncated, full)?.result).toEqual(full);
  });

  it("成对 Markdown 包含元信息、请求和响应", () => {
    const text = buildPairMarkdown(record());
    expect(text).toContain("### MCP 调用记录");
    expect(text).toContain("服务器：mock（stdio）");
    expect(text).toContain("方法：tools/call · echo");
    expect(text).toContain("**请求**");
    expect(text).toContain('"method": "tools/call"');
    expect(text).toContain("**响应**");
    expect(text).toContain('"jsonrpc": "2.0"');
  });

  it("成对 Markdown 对取消、应用层错误和探测记录给出明确说明", () => {
    expect(
      buildPairMarkdown(
        record({ status: "canceled", result: null, error: null }),
      ),
    ).toContain("调用已取消，无响应。");

    expect(
      buildPairMarkdown(
        record({
          status: "error",
          result: null,
          error: { kind: "timeout", message: "超时" },
        }),
      ),
    ).toContain("应用层错误（没有 JSON-RPC 响应）");

    const probeText = buildPairMarkdown(
      record({ method: "probe", target: "probe", params: {} }),
    );
    expect(probeText).toContain("探测记录没有标准 JSON-RPC 请求");
    expect(probeText).toContain("探测结果（应用层）");
  });

  it("Markdown 代码围栏随内容中的反引号加长", () => {
    const text = buildPairMarkdown(
      record({ result: { content: [{ type: "text", text: "```" }] } }),
    );
    expect(text).toContain("````json");
  });

  it("成对 JSON 保留 response、result 和应用层 error", () => {
    const success = JSON.parse(buildPairJson(record()));
    expect(success.meta).toMatchObject({
      record_id: "c1",
      server: "mock",
      method: "tools/call",
      status: "ok",
    });
    expect(success.request.method).toBe("tools/call");
    expect(success.response.result).toEqual(record().result);
    expect(success.result).toEqual(record().result);
    expect(success.app_error).toBeNull();

    const failed = JSON.parse(
      buildPairJson(
        record({
          status: "error",
          result: null,
          error: { kind: "timeout", message: "超时" },
        }),
      ),
    );
    expect(failed.response).toBeNull();
    expect(failed.result).toBeNull();
    expect(failed.app_error.kind).toBe("timeout");
  });
});
