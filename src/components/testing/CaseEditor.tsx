import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { createTestCases, updateTestCase } from "../../api/tests";
import { toast } from "../../stores/toast";
import type {
  AssertionKind,
  AssertionSeverity,
  TestAssertion,
  TestCase,
  TestCaseDraft,
  TestExpect,
  TestMethod,
  TestOutcome,
  TestPriority,
  TestRisk,
} from "../../types/testing";
import { Field, Modal } from "./SuiteDialog";

interface Props {
  suiteId: string;
  testCase?: TestCase;
  onClose: () => void;
  onSaved: () => void;
}

const METHODS: { value: TestMethod; label: string }[] = [
  { value: "tools_call", label: "tools/call" },
  { value: "resources_read", label: "resources/read" },
  { value: "prompts_get", label: "prompts/get" },
];

const OUTCOMES: { value: TestOutcome; label: string }[] = [
  { value: "success", label: "应成功" },
  { value: "tool_error", label: "应返回工具错误" },
  { value: "rpc_error", label: "应返回协议错误" },
  { value: "rejected", label: "应被拒绝（工具/协议错误均可）" },
  { value: "any", label: "仅观察（不判期望）" },
];

const ASSERTION_EXAMPLE =
  '{"id":"a1","kind":"json_path","severity":"error","enabled":true,"path":"$.structuredContent.ok","op":"eq","expected":true,"ignore_paths":[]}';

export function CaseEditor({ suiteId, testCase, onClose, onSaved }: Props) {
  const [name, setName] = useState(testCase?.name ?? "");
  const [description, setDescription] = useState(testCase?.description ?? "");
  const [method, setMethod] = useState<TestMethod>(
    testCase?.method ?? "tools_call",
  );
  const [target, setTarget] = useState(testCase?.target ?? "");
  const [argumentsText, setArgumentsText] = useState(
    JSON.stringify(testCase?.arguments ?? {}, null, 2),
  );
  const [outcome, setOutcome] = useState<TestOutcome>(
    testCase?.expect.outcome ?? "success",
  );
  const [acceptAsWarning, setAcceptAsWarning] = useState(
    testCase?.expect.accept_as_warning ?? true,
  );
  const [assertionsText, setAssertionsText] = useState(
    JSON.stringify(testCase?.assertions ?? [], null, 2),
  );
  const [editorTab, setEditorTab] = useState<"arguments" | "assertions">(
    "arguments",
  );
  const [tags, setTags] = useState((testCase?.tags ?? []).join(", "));
  const [priority, setPriority] = useState<TestPriority>(
    testCase?.priority ?? "p1",
  );
  const [risk, setRisk] = useState<TestRisk>(testCase?.risk ?? "unknown");
  const [enabled, setEnabled] = useState(testCase?.enabled ?? true);
  const [timeoutSecs, setTimeoutSecs] = useState(
    testCase?.timeout_secs ? String(testCase.timeout_secs) : "",
  );
  const [retryCount, setRetryCount] = useState(
    String(testCase?.retry_count ?? 0),
  );

  const save = useMutation({
    mutationFn: async () => {
      const draft = buildDraft();
      if (testCase) return updateTestCase(testCase.id, draft);
      const created = await createTestCases(suiteId, [draft]);
      return created[0];
    },
    onSuccess: () => {
      toast.success(testCase ? "用例已更新" : "用例已创建");
      onSaved();
      onClose();
    },
    onError: (error) => toast.fromError(error, "保存用例失败"),
  });

  function buildDraft(): TestCaseDraft {
    let parsedArguments: unknown;
    let parsedAssertions: TestAssertion[];
    try {
      parsedArguments = JSON.parse(argumentsText || "{}");
    } catch (error) {
      // 错误信息已包含原始解析错误；保持 ES2020 目标，不使用 Error cause 选项。
      // eslint-disable-next-line preserve-caught-error
      throw new Error(`参数 JSON 不合法：${String(error)}`);
    }
    try {
      const value = JSON.parse(assertionsText || "[]");
      if (!Array.isArray(value)) throw new Error("断言必须是数组");
      parsedAssertions = value as TestAssertion[];
    } catch (error) {
      // eslint-disable-next-line preserve-caught-error
      throw new Error(`断言 JSON 不合法：${String(error)}`);
    }
    const expect: TestExpect = {
      outcome,
      accept_as_warning: acceptAsWarning,
    };
    return {
      name: name.trim(),
      description: description.trim(),
      method,
      target: target.trim(),
      arguments: parsedArguments,
      expect,
      assertions: parsedAssertions,
      tags: tags
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean),
      priority,
      risk,
      source: testCase?.source ?? "manual",
      generation_note: testCase?.generation_note ?? null,
      enabled,
      timeout_secs: timeoutSecs ? Number(timeoutSecs) : null,
      retry_count: Number(retryCount) || 0,
    };
  }

  const argsInvalid = !isJson(argumentsText);
  const assertionsInvalid = !isJsonArray(assertionsText);
  const saveDisabledReason = save.isPending
    ? "保存中…"
    : !name.trim()
      ? "名称不能为空"
      : !target.trim()
        ? "目标不能为空"
        : argsInvalid
          ? "参数不是合法 JSON"
          : assertionsInvalid
            ? "断言必须是合法 JSON 数组"
            : undefined;

  const activeInvalid =
    editorTab === "arguments" ? argsInvalid : assertionsInvalid;

  return (
    <Modal
      title={testCase ? "编辑用例" : "新建用例"}
      wide
      onClose={onClose}
      footer={
        <div className="flex items-center gap-2">
          <span
            title={saveDisabledReason}
            className={`mr-auto min-w-0 truncate text-[11px] ${
              saveDisabledReason ? "text-red-600" : "text-slate-400"
            }`}
          >
            {saveDisabledReason ?? "参数与断言校验通过"}
          </span>
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
            onClick={onClose}
          >
            取消
          </button>
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={Boolean(saveDisabledReason)}
            title={saveDisabledReason}
            onClick={() => save.mutate()}
          >
            {save.isPending ? "保存中…" : "保存"}
          </button>
        </div>
      }
    >
      <div className="grid gap-4 md:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)]">
        <div className="min-w-0 space-y-3">
          <div className="grid grid-cols-2 gap-3 text-xs">
            <div className="col-span-2">
              <Field label="名称">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </Field>
            </div>
            <div className="col-span-2">
              <Field label="目标">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={target}
                  onChange={(event) => setTarget(event.target.value)}
                  placeholder="工具名 / 资源 URI / 提示词名"
                />
              </Field>
            </div>
            <Field label="方法">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={method}
                onChange={(event) =>
                  setMethod(event.target.value as TestMethod)
                }
              >
                {METHODS.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="期望">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={outcome}
                onChange={(event) =>
                  setOutcome(event.target.value as TestOutcome)
                }
              >
                {OUTCOMES.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </select>
            </Field>
            <div className="col-span-2">
              <Field label="描述">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={description}
                  onChange={(event) => setDescription(event.target.value)}
                />
              </Field>
            </div>
            <div className="col-span-2">
              <Field label="标签（逗号分隔）">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={tags}
                  onChange={(event) => setTags(event.target.value)}
                />
              </Field>
            </div>
            <Field label="优先级">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={priority}
                onChange={(event) =>
                  setPriority(event.target.value as TestPriority)
                }
              >
                <option value="p0">P0</option>
                <option value="p1">P1</option>
                <option value="p2">P2</option>
              </select>
            </Field>
            <Field label="风险">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={risk}
                onChange={(event) => setRisk(event.target.value as TestRisk)}
              >
                <option value="read_only">只读</option>
                <option value="write">写</option>
                <option value="unknown">未知</option>
                <option value="destructive">破坏性</option>
              </select>
            </Field>
          </div>

          <details className="rounded border border-slate-200 bg-slate-50/50">
            <summary className="cursor-pointer select-none px-3 py-1.5 text-xs font-medium text-slate-500">
              高级执行设置
              <span className="ml-2 font-normal text-[11px] text-slate-400">
                超时{" "}
                {timeoutSecs.trim() ? `${timeoutSecs} 秒` : "默认"} · 重试{" "}
                {retryCount.trim() || "0"} 次 · {enabled ? "已启用" : "已停用"}
              </span>
            </summary>
            <div className="grid grid-cols-2 gap-3 border-t border-slate-100 p-3 text-xs">
              <Field label="超时（秒，可空）">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={timeoutSecs}
                  onChange={(event) => setTimeoutSecs(event.target.value)}
                  inputMode="numeric"
                />
              </Field>
              <Field label="重试次数（仅环境错误）">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={retryCount}
                  onChange={(event) => setRetryCount(event.target.value)}
                  inputMode="numeric"
                />
              </Field>
              <label className="col-span-2 flex items-center gap-2 text-xs text-slate-600">
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(event) => setEnabled(event.target.checked)}
                />
                启用
              </label>
              <label className="col-span-2 flex items-center gap-2 text-xs text-slate-600">
                <input
                  type="checkbox"
                  checked={acceptAsWarning}
                  onChange={(event) => setAcceptAsWarning(event.target.checked)}
                />
                负向被接受时仅警告
              </label>
            </div>
          </details>
        </div>

        <div className="flex min-w-0 flex-col gap-2">
          <div className="inline-flex self-start overflow-hidden rounded border border-slate-200">
            <button
              type="button"
              aria-pressed={editorTab === "arguments"}
              className={`px-2 py-1 text-[11px] ${
                editorTab === "arguments"
                  ? "bg-slate-800 text-white"
                  : "bg-white text-slate-600 hover:bg-slate-50"
              }`}
              onClick={() => setEditorTab("arguments")}
            >
              参数（JSON）
              {argsInvalid ? (
                <span className="ml-1 text-red-400">!</span>
              ) : null}
            </button>
            <button
              type="button"
              aria-pressed={editorTab === "assertions"}
              className={`px-2 py-1 text-[11px] ${
                editorTab === "assertions"
                  ? "bg-slate-800 text-white"
                  : "bg-white text-slate-600 hover:bg-slate-50"
              }`}
              onClick={() => setEditorTab("assertions")}
            >
              断言（JSON 数组）
              {assertionsInvalid ? (
                <span className="ml-1 text-red-400">!</span>
              ) : null}
            </button>
          </div>
          <textarea
            aria-label={
              editorTab === "arguments" ? "参数（JSON）" : "断言（JSON 数组）"
            }
            className={`h-[min(38vh,360px)] w-full resize-none rounded border px-2 py-1.5 font-mono text-xs ${
              activeInvalid ? "border-red-300" : "border-slate-200"
            }`}
            value={
              editorTab === "arguments" ? argumentsText : assertionsText
            }
            onChange={(event) =>
              editorTab === "arguments"
                ? setArgumentsText(event.target.value)
                : setAssertionsText(event.target.value)
            }
          />
          <div className="min-h-[18px] text-[11px]">
            {activeInvalid ? (
              <p className="text-red-500">
                {editorTab === "arguments"
                  ? "参数不是合法 JSON"
                  : "断言必须是合法 JSON 数组"}
              </p>
            ) : (
              <p className="text-slate-400">
                {editorTab === "arguments"
                  ? "参数按 JSON 对象校验，为空时等价于 {}。"
                  : "断言必须是 JSON 数组，可展开下方示例。"}
              </p>
            )}
          </div>
          {editorTab === "assertions" ? (
            <details className="rounded border border-slate-200 bg-slate-50/50 text-[11px] text-slate-500">
              <summary className="cursor-pointer select-none px-2 py-1">
                断言示例
              </summary>
              <code className="block break-all border-t border-slate-100 px-2 py-1.5 font-mono text-[10px] text-slate-600">
                {ASSERTION_EXAMPLE}
              </code>
            </details>
          ) : (
            <p className="text-[11px] text-slate-400">
              M1 使用 JSON 编辑；M3 将提供可视化断言编辑器。
            </p>
          )}
        </div>
      </div>
    </Modal>
  );
}

function isJson(value: string): boolean {
  try {
    JSON.parse(value || "{}");
    return true;
  } catch {
    return false;
  }
}

function isJsonArray(value: string): boolean {
  try {
    return Array.isArray(JSON.parse(value || "[]"));
  } catch {
    return false;
  }
}

/** 保留给后续可视化断言编辑器使用的联合类型导出。 */
export type { AssertionKind, AssertionSeverity };
