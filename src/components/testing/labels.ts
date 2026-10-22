/**
 * 测试报告文案（v0.2.2）
 *
 * 运行状态与用例状态分开维护；状态名、颜色、失败分类、跳过原因与错误标签
 * 由 Reports / RunCaseDetail 共用，避免两套不一致的文案。
 */

import type { CallError } from "../../types/models";
import type {
  FailureKind,
  TestRunCaseStatus,
  TestRunStatus,
} from "../../types/testing";

export const RUN_STATUS_LABEL: Record<TestRunStatus, string> = {
  running: "运行中",
  completed: "完成",
  failed: "失败",
  canceled: "取消",
  interrupted: "中断",
};

export const RUN_STATUS_CLASS: Record<TestRunStatus, string> = {
  running: "text-sky-600",
  completed: "text-emerald-600",
  failed: "text-red-600",
  canceled: "text-slate-500",
  interrupted: "text-amber-600",
};

export const CASE_STATUS_LABEL: Record<TestRunCaseStatus, string> = {
  passed: "通过",
  failed: "失败",
  warning: "警告",
  skipped: "跳过",
  error: "错误",
  canceled: "取消",
};

export const CASE_STATUS_CLASS: Record<TestRunCaseStatus, string> = {
  passed: "text-emerald-600",
  failed: "text-red-600",
  warning: "text-amber-600",
  skipped: "text-slate-400",
  error: "text-red-700",
  canceled: "text-slate-500",
};

export const CASE_STATUS_FILTERS: {
  value: TestRunCaseStatus;
  label: string;
}[] = [
  { value: "passed", label: CASE_STATUS_LABEL.passed },
  { value: "failed", label: CASE_STATUS_LABEL.failed },
  { value: "warning", label: CASE_STATUS_LABEL.warning },
  { value: "skipped", label: CASE_STATUS_LABEL.skipped },
  { value: "error", label: CASE_STATUS_LABEL.error },
  { value: "canceled", label: CASE_STATUS_LABEL.canceled },
];

export const FAILURE_KIND_LABEL: Record<FailureKind, string> = {
  assertion_failed: "断言失败",
  invalid_input_accepted: "非法输入被接受",
  protocol_error: "协议错误",
  tool_error_unexpected: "非预期工具错误",
  transport_error: "传输错误",
  timeout: "超时",
  server_unhealthy: "服务不健康",
  case_invalid: "用例无效",
  unknown: "未知",
};

export const SKIP_REASON_LABEL: Record<string, string> = {
  disabled: "用例已禁用",
  write_not_allowed: "未允许写入",
  unknown_not_allowed: "未允许未知风险",
  destructive_not_allowed: "未允许破坏性操作",
  target_missing: "目标不存在",
  after_cancel: "运行取消后跳过",
  after_failure: "失败后跳过",
  skipped: "已跳过",
};

export function skipReasonLabel(reason?: string | null): string {
  if (!reason) return "已跳过";
  return SKIP_REASON_LABEL[reason] ?? "已跳过（未知原因）";
}

export function callErrorLabel(error: CallError): string {
  switch (error.kind) {
    case "timeout":
      return "调用超时";
    case "rpc_error":
      return error.code !== undefined && error.code !== null
        ? `服务端返回错误（${error.code}）`
        : "服务端返回错误";
    case "transport_closed":
      return "传输已关闭";
    case "validation":
      return "参数校验失败";
    case "capability_missing":
      return "服务器未声明该能力";
    case "input_required":
      return "需要交互式输入";
    case "internal":
      return "内部错误";
    default:
      return error.kind;
  }
}
