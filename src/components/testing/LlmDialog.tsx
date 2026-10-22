import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { exportLlmPrompt, importLlmTestCases } from "../../api/tests";
import { toast } from "../../stores/toast";
import type { GenerationOptions, ImportTestCasesReport } from "../../types/testing";
import { Modal } from "./SuiteDialog";

interface Props {
  suiteId: string;
  serverId: string;
  onClose: () => void;
  onSaved: () => void;
}

const OPTIONS: GenerationOptions = {
  scope: "all",
  profile: "standard",
  include_boundary: true,
  include_negative: true,
  include_resources: true,
  include_prompts: true,
  use_history: false,
  allow_destructive: false,
  max_cases_per_tool: 12,
  max_total: 30,
  seed: 20261021,
};

/** LLM 提示词导出 + 用例 JSON 导入（不调用模型 API）。 */
export function LlmDialog({ suiteId, serverId, onClose, onSaved }: Props) {
  const [prompt, setPrompt] = useState("");
  const [json, setJson] = useState("");
  const [report, setReport] = useState<ImportTestCasesReport | null>(null);

  const generate = useMutation({
    mutationFn: () => exportLlmPrompt(serverId, OPTIONS),
    onSuccess: (text) => {
      setPrompt(text);
      toast.success("提示词已生成");
    },
    onError: (error) => toast.fromError(error, "生成提示词失败"),
  });

  const importCases = useMutation({
    mutationFn: () => importLlmTestCases(suiteId, json),
    onSuccess: (value) => {
      setReport(value);
      if (value.added > 0) {
        toast.success(`已导入 ${value.added} 条，需人工复核后启用`);
        onSaved();
      } else {
        toast.error("没有可导入的用例");
      }
    },
    onError: (error) => toast.fromError(error, "导入失败"),
  });

  return (
    <Modal
      title="LLM 辅助生成（提示词导出 / 用例导入）"
      wide
      onClose={onClose}
    >
      <div className="space-y-4 text-xs">
        <p className="rounded border border-amber-200 bg-amber-50 px-3 py-2 text-amber-700">
          提示词只包含接口定义，不含凭据/历史参数。LLM 生成结果必须导入后人工复核，
          默认不启用；本版本不调用任何模型 API。
        </p>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={generate.isPending}
            onClick={() => generate.mutate()}
          >
            {generate.isPending ? "生成中…" : "生成提示词"}
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 hover:bg-slate-50 disabled:opacity-50"
            disabled={!prompt}
            onClick={() => {
              void navigator.clipboard
                .writeText(prompt)
                .then(() => toast.success("提示词已复制"))
                .catch((error) => toast.fromError(error, "复制失败"));
            }}
          >
            复制
          </button>
          <span className="text-slate-400">
            将提示词交给任意 LLM，让它输出用例 JSON。
          </span>
        </div>
        <textarea
          className="h-56 w-full rounded border border-slate-200 px-2 py-1.5 font-mono text-[11px]"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          placeholder="点击“生成提示词”后显示；可手动编辑"
        />
        <div>
          <div className="mb-1 text-slate-500">导入 LLM 用例 JSON</div>
          <textarea
            className="h-40 w-full rounded border border-slate-200 px-2 py-1.5 font-mono text-[11px]"
            value={json}
            onChange={(event) => setJson(event.target.value)}
            placeholder='{ "schema_version": 1, "cases": [ ... ] }'
          />
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded bg-emerald-600 px-3 py-1.5 text-white hover:bg-emerald-500 disabled:opacity-50"
            disabled={!json.trim() || importCases.isPending}
            onClick={() => importCases.mutate()}
          >
            {importCases.isPending ? "导入中…" : "校验并导入"}
          </button>
          {report ? (
            <span className="text-slate-500">
              新增 {report.added} · 失败 {report.failed.length}
            </span>
          ) : null}
        </div>
        {report && report.failed.length > 0 ? (
          <ul className="max-h-32 list-disc overflow-y-auto pl-5 text-red-600">
            {report.failed.map((item) => (
              <li key={item.index}>
                第 {item.index + 1} 条{item.name ? `（${item.name}）` : ""}：
                {item.reason}
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </Modal>
  );
}
