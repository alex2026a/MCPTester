import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { generateTestCases, saveGeneratedCases } from "../../api/tests";
import { toast } from "../../stores/toast";
import type {
  GenerationOptions,
  GenerationProfile,
  GenerationPreview,
  TestCaseDraft,
} from "../../types/testing";
import { Field, Modal } from "./SuiteDialog";

interface Props {
  suiteId: string;
  serverId: string;
  target?: string | null;
  onClose: () => void;
  onSaved: () => void;
}

const PROFILE_LABEL: Record<GenerationProfile, string> = {
  smoke: "快速冒烟",
  standard: "标准回归",
  thorough: "彻底检查",
};

/** 生成预览 → 勾选 → 原子保存；不自动执行。 */
export function GenerationDialog({
  suiteId,
  serverId,
  target,
  onClose,
  onSaved,
}: Props) {
  const [profile, setProfile] = useState<GenerationProfile>("standard");
  const [includeBoundary, setIncludeBoundary] = useState(true);
  const [includeNegative, setIncludeNegative] = useState(true);
  const [includeResources, setIncludeResources] = useState(true);
  const [includePrompts, setIncludePrompts] = useState(true);
  const [useHistory, setUseHistory] = useState(true);
  const [allowDestructive, setAllowDestructive] = useState(false);
  const [maxCasesPerTool, setMaxCasesPerTool] = useState(12);
  const [maxTotal, setMaxTotal] = useState(300);
  const [preview, setPreview] = useState<GenerationPreview | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());

  const options: GenerationOptions = useMemo(
    () => ({
      target: target ?? null,
      scope: target ? "current" : "all",
      profile,
      include_boundary: includeBoundary,
      include_negative: includeNegative,
      include_resources: includeResources,
      include_prompts: includePrompts,
      use_history: useHistory,
      allow_destructive: allowDestructive,
      max_cases_per_tool: maxCasesPerTool,
      max_total: maxTotal,
      seed: 20261021,
    }),
    [
      target,
      profile,
      includeBoundary,
      includeNegative,
      includeResources,
      includePrompts,
      useHistory,
      allowDestructive,
      maxCasesPerTool,
      maxTotal,
    ],
  );

  const generate = useMutation({
    mutationFn: () => generateTestCases(serverId, options),
    onSuccess: (result) => {
      setPreview(result);
      setSelected(new Set(result.cases.map((_, index) => index)));
      if (result.warnings.length > 0) {
        toast.info?.(`生成完成，有 ${result.warnings.length} 条提示`);
      }
    },
    onError: (error) => toast.fromError(error, "生成用例失败"),
  });

  const save = useMutation({
    mutationFn: async () => {
      if (!preview) throw new Error("请先生成预览");
      const cases: TestCaseDraft[] = preview.cases.filter((_, index) =>
        selected.has(index),
      );
      if (cases.length === 0) throw new Error("请至少选择一条用例");
      return saveGeneratedCases(
        suiteId,
        cases,
        preview.capability_hash,
        preview.capability_snapshot,
      );
    },
    onSuccess: (cases) => {
      toast.success(`已保存 ${cases.length} 条用例`);
      onSaved();
      onClose();
    },
    onError: (error) => toast.fromError(error, "保存生成用例失败"),
  });

  function toggle(index: number) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }

  return (
    <Modal title={target ? `为 ${target} 生成用例` : "生成测试用例"} wide onClose={onClose}>
      <div className="grid grid-cols-4 gap-3 text-xs">
        <Field label="档位">
          <select
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={profile}
            onChange={(event) =>
              setProfile(event.target.value as GenerationProfile)
            }
          >
            {Object.entries(PROFILE_LABEL).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="每工具上限">
          <input
            type="number"
            min={1}
            max={50}
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={maxCasesPerTool}
            onChange={(event) => setMaxCasesPerTool(Number(event.target.value))}
          />
        </Field>
        <Field label="整套上限">
          <input
            type="number"
            min={1}
            max={1000}
            className="w-full rounded border border-slate-200 px-2 py-1.5"
            value={maxTotal}
            onChange={(event) => setMaxTotal(Number(event.target.value))}
          />
        </Field>
        <Field label="随机种子">
          <input
            className="w-full rounded border border-slate-200 bg-slate-50 px-2 py-1.5"
            value={20261021}
            readOnly
          />
        </Field>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={includeBoundary}
            onChange={(event) => setIncludeBoundary(event.target.checked)}
          />
          边界
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={includeNegative}
            onChange={(event) => setIncludeNegative(event.target.checked)}
          />
          负向
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={includeResources}
            onChange={(event) => setIncludeResources(event.target.checked)}
          />
          资源
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={includePrompts}
            onChange={(event) => setIncludePrompts(event.target.checked)}
          />
          提示词
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={useHistory}
            onChange={(event) => setUseHistory(event.target.checked)}
          />
          使用历史成功参数
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={allowDestructive}
            onChange={(event) => setAllowDestructive(event.target.checked)}
          />
          启用破坏性用例
        </label>
        <div className="col-span-2 flex items-end justify-end gap-2">
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={generate.isPending}
            onClick={() => generate.mutate()}
          >
            {generate.isPending ? "生成中…" : preview ? "重新生成" : "生成预览"}
          </button>
        </div>
      </div>

      {preview ? (
        <div className="mt-4 space-y-2">
          <div className="flex flex-wrap items-center gap-3 rounded border border-slate-200 bg-slate-50 px-3 py-2 text-xs">
            <span>共 {preview.stats.total} 条</span>
            <span className="text-emerald-600">正向 {preview.stats.positive}</span>
            <span className="text-sky-600">边界 {preview.stats.boundary}</span>
            <span className="text-amber-600">负向 {preview.stats.negative}</span>
            <span className="text-slate-500">
              资源 {preview.stats.resources} · 提示词 {preview.stats.prompts}
            </span>
            <span className="ml-auto text-slate-500">
              已选 {selected.size} / {preview.cases.length}
            </span>
            <button
              type="button"
              className="rounded border border-slate-200 bg-white px-2 py-1 hover:bg-slate-50"
              onClick={() =>
                setSelected(
                  selected.size === preview.cases.length
                    ? new Set()
                    : new Set(preview.cases.map((_, index) => index)),
                )
              }
            >
              {selected.size === preview.cases.length ? "全不选" : "全选"}
            </button>
            <button
              type="button"
              className="rounded bg-emerald-600 px-3 py-1 text-white hover:bg-emerald-500 disabled:opacity-50"
              disabled={selected.size === 0 || save.isPending}
              onClick={() => save.mutate()}
            >
              {save.isPending ? "保存中…" : "接受所选"}
            </button>
          </div>

          {preview.warnings.length > 0 ? (
            <details className="rounded border border-amber-200 bg-amber-50 px-3 py-2 text-[11px] text-amber-700">
              <summary className="cursor-pointer">
                生成提示（{preview.warnings.length}）
              </summary>
              <ul className="mt-1 list-disc space-y-0.5 pl-4">
                {preview.warnings.map((warning, index) => (
                  <li key={index}>{warning}</li>
                ))}
              </ul>
            </details>
          ) : null}

          <div className="max-h-[45vh] overflow-y-auto rounded border border-slate-200">
            <table className="w-full text-left text-xs">
              <thead className="sticky top-0 bg-slate-50 text-slate-500">
                <tr>
                  <th className="w-8 px-2 py-2" />
                  <th className="px-2 py-2">名称</th>
                  <th className="px-2 py-2">方法 / 目标</th>
                  <th className="px-2 py-2">风险</th>
                  <th className="px-2 py-2">标签</th>
                </tr>
              </thead>
              <tbody>
                {preview.cases.map((item, index) => (
                  <tr key={index} className="border-t border-slate-100">
                    <td className="px-2 py-1.5">
                      <input
                        type="checkbox"
                        checked={selected.has(index)}
                        onChange={() => toggle(index)}
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <div className="text-slate-800">{item.name}</div>
                      {item.generation_note ? (
                        <div className="text-[10px] text-slate-400">
                          {item.generation_note}
                        </div>
                      ) : null}
                    </td>
                    <td className="px-2 py-1.5 text-slate-500">
                      <div>{item.method}</div>
                      <div className="max-w-48 truncate">{item.target}</div>
                    </td>
                    <td className="px-2 py-1.5 text-slate-500">{item.risk}</td>
                    <td className="px-2 py-1.5 text-slate-500">
                      {item.tags.join(", ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : (
        <div className="mt-4 rounded border border-dashed border-slate-200 p-6 text-center text-xs text-slate-400">
          选择档位后点击“生成预览”；预览不会写入数据库。
        </div>
      )}
    </Modal>
  );
}
