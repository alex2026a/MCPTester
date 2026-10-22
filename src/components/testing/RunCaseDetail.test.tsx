import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { RunCaseDetail } from "./RunCaseDetail";
import type { TestRunCaseDetail } from "../../types/testing";

function detail(overrides: Partial<TestRunCaseDetail> = {}): TestRunCaseDetail {
  return {
    summary: {
      id: "rc-1",
      case_id: "c-1",
      case_name: "调用 echo",
      method: "tools_call",
      target: "echo",
      order_index: 0,
      status: "passed",
      failure_kind: null,
      skip_reason: null,
      duration_ms: 12,
      attempts: 1,
      call_record_id: "call-1",
      message: null,
    },
    case_snapshot: {
      method: "tools_call",
      target: "echo",
      arguments: { path: "/tmp" },
    },
    attempt_record_ids: ["call-1"],
    assertions: [],
    evidence: {
      request: { path: "/tmp" },
      response_preview: { content: [{ type: "text", text: "ok" }] },
      response_error: null,
      truncated: false,
      wire: [],
      notifications: [],
      stderr: [],
      omitted: false,
    },
    ...overrides,
  };
}

describe("RunCaseDetail", () => {
  it("把请求和响应分开显示，并使用快照里的参数", () => {
    render(<RunCaseDetail detail={detail()} />);
    expect(screen.getByText("请求")).toBeInTheDocument();
    expect(screen.getByText("响应")).toBeInTheDocument();
    expect(screen.getByText("tools_call")).toBeInTheDocument();
    expect(screen.getByText("echo")).toBeInTheDocument();
    expect(screen.getByText(/\/tmp/)).toBeInTheDocument();
  });

  it("工具 isError 通过时不显示红色失败文案", () => {
    render(
      <RunCaseDetail
        detail={detail({
          evidence: {
            ...detail().evidence,
            response_preview: {
              content: [{ type: "text", text: "boom" }],
              isError: true,
            },
          },
        })}
      />,
    );
    expect(
      screen.getByText("工具返回错误（isError = true，符合用例期望）"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("工具返回错误（isError = true）", { exact: true }),
    ).not.toBeInTheDocument();
  });

  it("失败调用在响应区显示结构化错误", () => {
    render(
      <RunCaseDetail
        detail={detail({
          summary: {
            ...detail().summary,
            status: "failed",
            failure_kind: "timeout",
            message: "调用超时",
          },
          evidence: {
            ...detail().evidence,
            response_preview: null,
            response_error: { kind: "timeout", timeout_secs: 5 },
          },
        })}
      />,
    );
    expect(screen.getAllByText("调用超时").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/超时/).length).toBeGreaterThan(0);
  });

  it("预期 RPC 错误通过时使用预期错误文案", () => {
    render(
      <RunCaseDetail
        detail={detail({
          evidence: {
            ...detail().evidence,
            response_preview: null,
            response_error: {
              kind: "rpc_error",
              code: -32602,
              message: "参数不合法",
            },
          },
        })}
      />,
    );
    expect(screen.getByText("预期协议错误")).toBeInTheDocument();
    expect(screen.queryByText(/^服务端返回错误/)).not.toBeInTheDocument();
  });

  it("跳过用例显示未执行，取消用例显示取消文案", () => {
    const { rerender } = render(
      <RunCaseDetail
        detail={detail({
          summary: {
            ...detail().summary,
            status: "skipped",
            skip_reason: "write_not_allowed",
            message: "跳过：write_not_allowed",
          },
        })}
      />,
    );
    expect(screen.getByText("用例未执行")).toBeInTheDocument();
    expect(screen.getByText(/未允许写入/)).toBeInTheDocument();

    rerender(
      <RunCaseDetail
        detail={detail({
          summary: {
            ...detail().summary,
            status: "canceled",
            message: "用例已取消",
          },
          evidence: {
            ...detail().evidence,
            response_preview: null,
          },
        })}
      />,
    );
    expect(screen.getAllByText(/运行被取消|用例已取消/).length).toBeGreaterThan(0);
  });

  it("多次尝试时提示仅显示最后一次", () => {
    render(
      <RunCaseDetail
        detail={detail({
          summary: { ...detail().summary, attempts: 3 },
        })}
      />,
    );
    expect(screen.getByText(/仅显示最后一次/)).toBeInTheDocument();
  });

  it("证据省略时显示提示", () => {
    render(
      <RunCaseDetail
        detail={detail({
          evidence: { ...detail().evidence, omitted: true },
        })}
      />,
    );
    expect(screen.getAllByText(/证据超过 16KB/).length).toBeGreaterThan(0);
  });

  it("报文页签展示配对后的请求响应", () => {
    render(
      <RunCaseDetail
        detail={detail({
          evidence: {
            ...detail().evidence,
            wire: [
              {
                seq: 1,
                direction: "client_to_server",
                kind: "request",
                method: "tools/call",
                rpc_id: "1",
                summary: "request",
                payload: { method: "tools/call" },
                truncated: false,
                bytes: 10,
                is_error: false,
                duration_ms: null,
                at: "2026-10-22T00:00:00Z",
              },
              {
                seq: 2,
                direction: "server_to_client",
                kind: "response",
                method: null,
                rpc_id: "1",
                summary: "response",
                payload: { result: { ok: true } },
                truncated: false,
                bytes: 12,
                is_error: false,
                duration_ms: 8,
                at: "2026-10-22T00:00:00.008Z",
              },
            ],
          },
        })}
      />,
    );
    expect(screen.getByText("报文 2")).toBeInTheDocument();
    expect(screen.getAllByText("tools/call").length).toBeGreaterThan(0);
  });
});
