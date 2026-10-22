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

  return (
    <Modal
      title={testCase ? "编辑用例" : "新建用例"}
      wide
      onClose={onClose}
    >
      <div className="grid grid-cols-2 gap-3 text-sm">
        <Field label="名称">
          <input
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </Field>
        <Field label="目标">
          <input
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={target}
            onChange={(event) => setTarget(event.target.value)}
            placeholder="工具名 / 资源 URI / 提示词名"
          />
        </Field>
        <Field label="方法">
          <select
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={method}
            onChange={(event) => setMethod(event.target.value as TestMethod)}
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
            onChange={(event) => setOutcome(event.target.value as TestOutcome)}
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
          <Field label="参数（JSON）">
            <textarea
              className={`h-40 w-full rounded border px-2 py-1.5 font-mono text-xs ${
                argsInvalid ? "border-red-300" : "border-slate-200"
              }`}
              value={argumentsText}
              onChange={(event) => setArgumentsText(event.target.value)}
            />
            {argsInvalid ? (
              <p className="mt-1 text-[11px] text-red-500">参数不是合法 JSON</p>
            ) : null}
          </Field>
        </div>
        <div className="col-span-2">
          <Field label="断言（JSON 数组）">
            <textarea
              className={`h-44 w-full rounded border px-2 py-1.5 font-mono text-xs ${
                assertionsInvalid ? "border-red-300" : "border-slate-200"
              }`}
              value={assertionsText}
              onChange={(event) => setAssertionsText(event.target.value)}
            />
            <p className="mt-1 text-[11px] text-slate-400">
              M1 使用 JSON 编辑；M3 提供可视化断言编辑器。示例：
              {`{"id":"a1","kind":"json_path","severity":"error","enabled":true,"path":"$.structuredContent.ok","op":"eq","expected":true,"ignore_paths":[]}`}
            </p>
            {assertionsInvalid ? (
              <p className="mt-1 text-[11px] text-red-500">
                断言必须是合法 JSON 数组
              </p>
            ) : null}
          </Field>
        </div>
        <Field label="标签（逗号分隔）">
          <input
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={tags}
            onChange={(event) => setTags(event.target.value)}
          />
        </Field>
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
        <label className="flex items-center gap-2 text-xs text-slate-600">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) => setEnabled(event.target.checked)}
          />
          启用
        </label>
        <label className="flex items-center gap-2 text-xs text-slate-600">
          <input
            type="checkbox"
            checked={acceptAsWarning}
            onChange={(event) => setAcceptAsWarning(event.target.checked)}
          />
          负向被接受时仅警告
        </label>
        <div className="col-span-2 flex justify-end gap-2 pt-2">
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
            disabled={
              save.isPending ||
              !name.trim() ||
              !target.trim() ||
              argsInvalid ||
              assertionsInvalid
            }
            onClick={() => save.mutate()}
          >
            {save.isPending ? "保存中…" : "保存"}
          </button>
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
