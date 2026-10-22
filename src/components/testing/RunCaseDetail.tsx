/**
 * 测试用例详情（v0.2.2）
 *
 * 失败原因前置；请求/响应分栏；报文、通知、stderr、用例快照独立页签。
 * 颜色只由 summary.status 决定，工具 isError / 预期协议错误不误标失败。
 */

import { useState } from "react";
import { MessageViewer } from "../json/MessageViewer";
import { formatClock } from "../json/format";
import { WireFrames } from "./WireFrames";
import {
  CASE_STATUS_CLASS,
  CASE_STATUS_LABEL,
  FAILURE_KIND_LABEL,
  callErrorLabel,
  skipReasonLabel,
} from "./labels";
import type { TestCase, TestRunCaseDetail } from "../../types/testing";

const FAILURE_STATUS = new Set(["failed", "error"]);

type EvidenceTab = "wire" | "notifications" | "stderr" | "snapshot";

export function RunCaseDetail({ detail }: { detail: TestRunCaseDetail }) {
  const summary = detail.summary;
  const evidence = detail.evidence;
  const [tab, setTab] = useState<EvidenceTab>("wire");

  const snapshot =
    detail.case_snapshot !== null &&
    typeof detail.case_snapshot === "object" &&
    !Array.isArray(detail.case_snapshot)
      ? (detail.case_snapshot as Partial<TestCase>)
      : null;
  const argumentsValue = snapshot?.arguments ?? evidence.request ?? null;
  const method = snapshot?.method ?? summary.method;
  const target = snapshot?.target ?? summary.target;
  const response = evidence.response_preview;
  const responseError = evidence.response_error ?? null;
  const hasResponse = response !== undefined && response !== null;
  const failed = FAILURE_STATUS.has(summary.status);
  const responseTone = failed
    ? "error"
    : summary.status === "warning"
      ? "warning"
      : "neutral";
  const wireFrames = evidence.wire.filter(
    (entry) => entry.kind === "request" || entry.kind === "response",
  ).length;

  return (
    <div className="space-y-3 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className={CASE_STATUS_CLASS[summary.status]}>
          {CASE_STATUS_LABEL[summary.status]}
        </span>
        <span className="font-medium">{summary.case_name}</span>
        <span className="text-slate-400">
          {summary.method} · {summary.target} ·{" "}
          {summary.attempts > 1
            ? `${summary.attempts} 次尝试，仅显示最后一次 · 最后一次耗时 ${summary.duration_ms}ms`
            : `${summary.duration_ms}ms`}
        </span>
      </div>

      {summary.failure_kind || summary.message ? (
        <div
          className={`rounded border px-2 py-1.5 ${
            failed
              ? "border-red-200 bg-red-50 text-red-700"
              : summary.status === "warning"
                ? "border-amber-200 bg-amber-50 text-amber-700"
                : "border-slate-200 bg-slate-50 text-slate-600"
          }`}
        >
          {summary.failure_kind ? (
            <span className="mr-2 rounded bg-white/70 px-1 py-0.5 font-mono text-[10px]">
              {FAILURE_KIND_LABEL[summary.failure_kind] ?? summary.failure_kind}
            </span>
          ) : null}
          <span className="break-all">
            {summary.status === "skipped"
              ? `跳过原因：${skipReasonLabel(summary.skip_reason ?? summary.message)}`
              : summary.status === "canceled"
                ? (summary.message ?? "运行被取消")
                : (summary.message ?? "用例失败")}
          </span>
        </div>
      ) : null}

      {evidence.omitted ? (
        <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1.5 text-amber-700">
          证据超过 16KB，报文 / 通知 / stderr 已省略；请求与响应仍保留。
        </div>
      ) : null}

      {detail.assertions.length > 0 ? (
        <div className="overflow-x-auto rounded border border-slate-200 bg-white">
          <table className="w-full text-left">
            <thead className="bg-slate-50 text-slate-500">
              <tr>
                <th className="px-2 py-1">断言</th>
                <th className="px-2 py-1">结果</th>
                <th className="px-2 py-1">期望</th>
                <th className="px-2 py-1">实际</th>
                <th className="px-2 py-1">说明</th>
              </tr>
            </thead>
            <tbody>
              {detail.assertions.map((assertion) => (
                <tr
                  key={assertion.assertion_id}
                  className={`border-t border-slate-100 ${
                    assertion.passed ? "" : "bg-red-50/50"
                  }`}
                >
                  <td className="px-2 py-1">{assertion.kind}</td>
                  <td
                    className={`px-2 py-1 ${
                      assertion.passed ? "text-emerald-600" : "text-red-600"
                    }`}
                  >
                    {assertion.passed ? "通过" : "失败"}
                  </td>
                  <td className="max-w-48 px-2 py-1">
                    <code title={jsonText(assertion.expected)}>
                      {compact(assertion.expected)}
                    </code>
                  </td>
                  <td className="max-w-48 px-2 py-1">
                    <code title={jsonText(assertion.actual)}>
                      {compact(assertion.actual)}
                    </code>
                  </td>
                  <td className="px-2 py-1 text-slate-500">
                    {assertion.message}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      <div className="grid gap-3 lg:grid-cols-2">
        <section className="rounded border border-slate-200 bg-white">
          <div className="border-b border-slate-100 px-2 py-1.5 font-medium text-slate-600">
            请求
          </div>
          <div className="space-y-1 p-2">
            <div className="flex flex-wrap items-center gap-2 text-[10px] text-slate-500">
              <span className="font-mono">{method}</span>
              <span className="font-mono">{target}</span>
              {summary.status === "skipped" ? (
                <span className="text-slate-400">用例未执行</span>
              ) : null}
            </div>
            {argumentsValue === null || argumentsValue === undefined ? (
              <div className="rounded border border-slate-100 bg-slate-50 px-2 py-3 text-[11px] text-slate-400">
                {snapshot ? "无参数" : "暂无快照"}
              </div>
            ) : (
              <MessageViewer
                key={`request-${summary.id}`}
                value={argumentsValue}
                defaultTab="tree"
                tabs={["tree", "raw"]}
                contentClassName="max-h-64 overflow-auto"
              />
            )}
          </div>
        </section>

        <section className="rounded border border-slate-200 bg-white">
          <div className="border-b border-slate-100 px-2 py-1.5 font-medium text-slate-600">
            响应
          </div>
          <div className="space-y-2 p-2">
            {responseError ? (
              <ResponseError
                value={responseError}
                tone={responseTone}
                expected={!failed}
              />
            ) : null}
            {hasResponse ? (
              <>
                <MessageViewer
                  key={`response-${summary.id}`}
                  value={response}
                  method={summary.method}
                  errorTone={responseTone}
                  defaultTab="rendered"
                  contentClassName="max-h-64 overflow-auto"
                />
              </>
            ) : !responseError ? (
              <div className="rounded border border-slate-100 bg-slate-50 px-2 py-3 text-[11px] text-slate-400">
                {responsePlaceholder(summary.status)}
              </div>
            ) : null}
            {evidence.truncated ? (
              <div className="text-[10px] text-amber-600">
                结果已截断入库，完整内容见调用记录。
              </div>
            ) : null}
          </div>
        </section>
      </div>

      <div className="rounded border border-slate-200 bg-white">
        <div className="flex flex-wrap items-center gap-1 border-b border-slate-100 p-2">
          {(
            [
              ["wire", `报文 ${wireFrames}`],
              ["notifications", `通知 ${evidence.notifications.length}`],
              ["stderr", `stderr ${evidence.stderr.length}`],
              ["snapshot", "用例快照"],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              type="button"
              className={`rounded px-2 py-0.5 text-[11px] ${
                tab === key
                  ? "bg-slate-800 text-white"
                  : "border border-slate-200 text-slate-600 hover:bg-slate-50"
              }`}
              aria-pressed={tab === key}
              onClick={() => setTab(key)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="p-2">
          {tab === "wire" ? (
            evidence.omitted ? (
              <div className="text-[11px] text-amber-600">
                证据超过 16KB，报文已省略。
              </div>
            ) : (
              <WireFrames entries={evidence.wire} />
            )
          ) : tab === "notifications" ? (
            <NotificationList
              items={evidence.notifications}
              omitted={evidence.omitted}
            />
          ) : tab === "stderr" ? (
            <StderrList items={evidence.stderr} omitted={evidence.omitted} />
          ) : snapshot ? (
            <MessageViewer
              key={`snapshot-${summary.id}`}
              value={detail.case_snapshot}
              defaultTab="tree"
              tabs={["tree", "raw"]}
              contentClassName="max-h-64 overflow-auto"
            />
          ) : (
            <div className="text-[11px] text-slate-400">暂无快照。</div>
          )}
        </div>
      </div>
    </div>
  );
}

function ResponseError({
  value,
  tone,
  expected,
}: {
  value: NonNullable<TestRunCaseDetail["evidence"]["response_error"]>;
  tone: "error" | "warning" | "neutral";
  expected: boolean;
}) {
  const className =
    tone === "error"
      ? "border-red-200 bg-red-50 text-red-700"
      : tone === "warning"
        ? "border-amber-200 bg-amber-50 text-amber-700"
        : "border-slate-200 bg-slate-50 text-slate-600";
  return (
    <div className={`rounded border px-2 py-1.5 ${className}`}>
      <div className="mb-1 flex flex-wrap items-center gap-2">
        <span className="font-medium">
          {expected ? "预期协议错误" : callErrorLabel(value)}
        </span>
        {value.message ? (
          <span className="break-all text-[10px] opacity-80">{value.message}</span>
        ) : null}
      </div>
      <MessageViewer
        value={value}
        defaultTab="tree"
        tabs={["tree", "raw"]}
        contentClassName="max-h-48 overflow-auto"
      />
    </div>
  );
}

function NotificationList({
  items,
  omitted,
}: {
  items: TestRunCaseDetail["evidence"]["notifications"];
  omitted: boolean;
}) {
  if (omitted) {
    return <div className="text-[11px] text-amber-600">证据超过 16KB，通知已省略。</div>;
  }
  if (items.length === 0) {
    return <div className="text-[11px] text-slate-400">暂无通知。</div>;
  }
  return (
    <div className="space-y-1">
      {items.map((item) => (
        <details key={item.seq} className="rounded border border-slate-100">
          <summary className="cursor-pointer px-2 py-1 text-[11px] text-slate-600">
            <span className="mr-2 font-mono text-slate-400">
              {formatClock(item.at)}
            </span>
            {item.method}
          </summary>
          <pre className="max-h-40 overflow-auto border-t border-slate-100 bg-slate-50 p-2 text-[10px] text-slate-700">
            {jsonText(item.params)}
          </pre>
        </details>
      ))}
    </div>
  );
}

function StderrList({
  items,
  omitted,
}: {
  items: TestRunCaseDetail["evidence"]["stderr"];
  omitted: boolean;
}) {
  if (omitted) {
    return <div className="text-[11px] text-amber-600">证据超过 16KB，stderr 已省略。</div>;
  }
  if (items.length === 0) {
    return <div className="text-[11px] text-slate-400">暂无 stderr 输出。</div>;
  }
  return (
    <div className="space-y-1 font-mono text-[10px]">
      {items.map((item) => (
        <div key={item.seq} className="flex gap-2">
          <span className="shrink-0 text-slate-300">{formatClock(item.at)}</span>
          <span className="break-all text-slate-600">{item.line}</span>
        </div>
      ))}
    </div>
  );
}

function responsePlaceholder(status: string): string {
  if (status === "skipped") return "用例未执行。";
  if (status === "canceled") return "运行被取消。";
  return "无响应体（服务端未返回）。";
}

function jsonText(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? "null";
  } catch {
    return String(value);
  }
}

function compact(value: unknown): string {
  if (value === undefined) return "-";
  const text = jsonText(value);
  return text.length > 160 ? `${text.slice(0, 160)}…` : text;
}
