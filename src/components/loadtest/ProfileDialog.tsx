import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { previewLoadTest, startLoadTest } from "../../api/loadtest";
import { toast } from "../../stores/toast";
import type { TestCase, TestRisk } from "../../types/testing";
import type {
  LoadProfile,
  LoadSessionMode,
  LoadStage,
} from "../../types/loadtest";
import {
  MAX_LOAD_OPERATIONS,
  SHARE_UNITS,
  equalShares,
  formatPercentValue,
  formatShareDraft,
  formatUnitsAsPercent,
  normalizeShares,
  parseShareDraft,
  sharesToPercents,
  sumShares,
  type ShareInputMode,
} from "../../lib/loadAllocation";
import { Field, Modal } from "../testing/SuiteDialog";

interface Props {
  serverId: string;
  suiteId: string;
  suiteName: string;
  cases: TestCase[];
  onClose: () => void;
  onStarted: (runId: string, profileName: string) => void;
}

const RISK_LABEL: Record<TestRisk, string> = {
  read_only: "只读",
  write: "写",
  unknown: "未知",
  destructive: "破坏性",
};

const RISK_CLASS: Record<TestRisk, string> = {
  read_only: "bg-emerald-50 text-emerald-600",
  write: "bg-amber-50 text-amber-700",
  unknown: "bg-slate-100 text-slate-600",
  destructive: "bg-red-50 text-red-600",
};

const MAX_TOTAL_SECS = 7200;

interface AllocationEntry {
  enabled: boolean;
  /** 原始输入草稿：百分比 "25.0"，权重 "250"。 */
  draft: string;
}

function initialEntries(cases: TestCase[]): Record<string, AllocationEntry> {
  const eligible = cases.filter(
    (item) => item.enabled && item.risk !== "destructive",
  );
  const selected = eligible.slice(0, MAX_LOAD_OPERATIONS);
  const shares = equalShares(selected.length);
  return Object.fromEntries(
    cases.map((item) => {
      const index = selected.findIndex((candidate) => candidate.id === item.id);
      return [
        item.id,
        {
          enabled: index >= 0,
          draft: index >= 0 ? formatShareDraft(shares[index], "percent") : "0.0",
        },
      ];
    }),
  );
}

function validateLocal(profile: LoadProfile): string | null {
  if (!profile.name.trim()) return "压测名称不能为空";
  if (profile.stages.length === 0) return "至少需要一个阶段";
  if (profile.stages.some((stage) => stage.duration_secs <= 0)) {
    return "阶段时长必须大于 0";
  }
  if (profile.stages.some((stage) => stage.target_vus <= 0)) {
    return "阶段目标 VU 必须大于 0";
  }
  const total = profile.stages.reduce(
    (sum, stage) => sum + stage.duration_secs,
    0,
  );
  if (total > MAX_TOTAL_SECS) return "总时长不能超过 7200 秒";
  if (profile.think_time_min_ms > profile.think_time_max_ms) {
    return "think time 最小值不能大于最大值";
  }
  if (
    profile.abort_error_rate !== null &&
    (!Number.isFinite(profile.abort_error_rate) ||
      profile.abort_error_rate < 0 ||
      profile.abort_error_rate > 1)
  ) {
    return "错误率中止阈值必须在 0–1";
  }
  if (
    profile.abort_p99_ms !== null &&
    (!Number.isFinite(profile.abort_p99_ms) || profile.abort_p99_ms <= 0)
  ) {
    return "p99 中止阈值必须大于 0";
  }
  if (profile.operations.length === 0) return "至少选择一个用例";
  return null;
}

/** 压测配置对话框：阶段、调用占比、风险授权。 */
export function ProfileDialog({
  serverId,
  suiteId,
  suiteName,
  cases,
  onClose,
  onStarted,
}: Props) {
  const [name, setName] = useState(`${suiteName} 压测`);
  const [sessionMode, setSessionMode] =
    useState<LoadSessionMode>("per_vu");
  const [stages, setStages] = useState<LoadStage[]>([
    { duration_secs: 60, target_vus: 10 },
  ]);
  const [thinkMin, setThinkMin] = useState(0);
  const [thinkMax, setThinkMax] = useState(0);
  const [timeoutSecs, setTimeoutSecs] = useState(30);
  const [abortErrorRate, setAbortErrorRate] = useState("0.2");
  const [abortP99, setAbortP99] = useState("");
  const [allowWrite, setAllowWrite] = useState(false);
  const [allowUnknown, setAllowUnknown] = useState(false);
  const [confirmProduction, setConfirmProduction] = useState(false);
  const [previewVersion, setPreviewVersion] = useState(0);
  const [mode, setMode] = useState<ShareInputMode>("percent");
  const [entries, setEntries] = useState<Record<string, AllocationEntry>>(() =>
    initialEntries(cases),
  );

  const previewQuery = useQuery({
    queryKey: ["load-preview", serverId, suiteId, previewVersion],
    queryFn: () => previewLoadTest(serverId, suiteId, buildProfile()),
    retry: false,
    refetchOnWindowFocus: false,
    placeholderData: (previous) => previous,
  });
  const preview = previewQuery.data ?? null;
  const previewError = previewQuery.error ? String(previewQuery.error) : null;

  function caseRisk(item: TestCase): TestRisk {
    return (
      preview?.operations.find((operation) => operation.case_id === item.id)
        ?.effective_risk ?? item.risk
    );
  }

  const selectedCases = cases.filter(
    (item) => entries[item.id]?.enabled && caseRisk(item) !== "destructive",
  );
  const selectedUnits = selectedCases.map((item) =>
    parseShareDraft(entries[item.id]?.draft ?? "", mode),
  );
  const totalUnits = sumShares(selectedUnits);
  const weightShares = sharesToPercents(selectedUnits);
  const weightPercentByCase = new Map<string, number>();
  selectedCases.forEach((item, index) => {
    weightPercentByCase.set(item.id, weightShares[index] ?? 0);
  });

  function allocationError(): string | null {
    if (selectedCases.length === 0) return null;
    if (selectedCases.length > MAX_LOAD_OPERATIONS) {
      return `最多选择 ${MAX_LOAD_OPERATIONS} 个用例（当前 ${selectedCases.length}）`;
    }
    if (selectedUnits.every((unit) => unit === 0)) {
      return mode === "percent"
        ? "已选用例占比不能全部为 0"
        : "已选用例权重不能全部为 0";
    }
    if (selectedUnits.some((unit) => unit < 1 || unit > SHARE_UNITS)) {
      return mode === "percent"
        ? "已选用例占比必须在 0.1%–100.0%"
        : "权重必须在 1–1000";
    }
    if (mode === "percent" && totalUnits !== SHARE_UNITS) {
      return `已选用例占比合计必须为 100.0%（当前 ${formatUnitsAsPercent(totalUnits)}）`;
    }
    return null;
  }

  function buildProfile(): LoadProfile {
    return {
      name: name.trim(),
      session_mode: sessionMode,
      stages,
      think_time_min_ms: thinkMin,
      think_time_max_ms: thinkMax,
      timeout_secs: timeoutSecs,
      operations: cases
        .filter(
          (item) =>
            entries[item.id]?.enabled &&
            caseRisk(item) !== "destructive",
        )
        .map((item) => ({
          case_id: item.id,
          weight: Math.min(
            SHARE_UNITS,
            Math.max(
              1,
              parseShareDraft(entries[item.id]?.draft ?? "", mode),
            ),
          ),
        })),
      allow_write: allowWrite,
      allow_unknown: allowUnknown,
      abort_error_rate: abortErrorRate.trim()
        ? Number(abortErrorRate)
        : null,
      abort_p99_ms: abortP99.trim() ? Number(abortP99) : null,
      seed: 20261112,
    };
  }

  const profile = buildProfile();
  const localError = validateLocal(profile) ?? allocationError();
  const previewOperations = preview?.operations ?? [];
  const riskReady =
    previewQuery.isSuccess &&
    !previewQuery.isError &&
    !previewQuery.isRefetchError &&
    !previewQuery.isFetching &&
    preview !== null &&
    selectedCases.every((item) => {
      const operation = previewOperations.find(
        (candidate) => candidate.case_id === item.id,
      );
      return operation?.allowed === true;
    });

  const start = useMutation({
    mutationFn: () =>
      startLoadTest(
        serverId,
        suiteId,
        buildProfile(),
        confirmProduction,
      ),
    onSuccess: (runId) => {
      toast.info("压测已开始");
      onStarted(runId, profile.name);
      onClose();
    },
    onError: (error) => toast.fromError(error, "启动压测失败"),
  });

  const productionNeedsConfirm =
    Boolean(preview?.production) &&
    (allowWrite || allowUnknown) &&
    !confirmProduction;
  const canStart =
    !localError &&
    !productionNeedsConfirm &&
    !start.isPending &&
    riskReady;
  const startDisabledReason =
    localError ??
    (previewQuery.isFetching
      ? "风险评估中…"
      : previewQuery.isError || previewQuery.isRefetchError
        ? "风险评估失败，请重试"
        : !riskReady
          ? "风险预览未就绪"
          : productionNeedsConfirm
            ? "请确认生产服务器压测授权"
            : undefined);

  const advancedSummary = `think ${thinkMin}–${thinkMax}ms · 超时 ${timeoutSecs}s · 错误率 ${abortErrorRate.trim() || "关闭"}`;
  const totalSecs = profile.stages.reduce(
    (sum, stage) => sum + stage.duration_secs,
    0,
  );
  const maxVus = Math.max(
    ...profile.stages.map((stage) => stage.target_vus),
    0,
  );
  const footerStatus =
    localError ??
    (previewQuery.isFetching
      ? "风险评估中…"
      : previewQuery.isError || previewQuery.isRefetchError
        ? "风险预览不可用，请重试"
        : !riskReady
          ? "风险预览未就绪"
          : productionNeedsConfirm
            ? "请确认生产服务器压测授权"
            : `${selectedCases.length} 个用例 · 总时长 ${totalSecs} 秒 · 最大 ${maxVus} VU`);

  const weightModeDisabled =
    selectedCases.length > 0 &&
    !(
      selectedUnits.every(
        (unit) => unit >= 1 && unit <= SHARE_UNITS,
      ) && totalUnits === SHARE_UNITS
    );
  const percentModeDisabled =
    selectedCases.length > 0 &&
    (totalUnits <= 0 || selectedUnits.some((unit) => unit < 0));
  const percentTotalValid =
    totalUnits === SHARE_UNITS &&
    selectedUnits.every(
      (unit) => unit >= 1 && unit <= SHARE_UNITS,
    );

  function updateStage(index: number, patch: Partial<LoadStage>) {
    setStages((current) =>
      current.map((stage, itemIndex) =>
        itemIndex === index ? { ...stage, ...patch } : stage,
      ),
    );
  }

  function updateDraft(id: string, draft: string) {
    setEntries((current) => {
      const entry = current[id];
      if (!entry) return current;
      return { ...current, [id]: { ...entry, draft } };
    });
  }

  function canonicalizeDraft(id: string) {
    setEntries((current) => {
      const entry = current[id];
      if (!entry) return current;
      const units = parseShareDraft(entry.draft, mode);
      return {
        ...current,
        [id]: { ...entry, draft: formatShareDraft(units, mode) },
      };
    });
  }

  function rebalance(
    current: Record<string, AllocationEntry>,
    options?: { equalizeZero?: boolean },
  ): Record<string, AllocationEntry> {
    const selected = cases.filter(
      (item) =>
        current[item.id]?.enabled &&
        caseRisk(item) !== "destructive",
    );
    if (selected.length === 0) return current;
    const units = selected.map((item) =>
      parseShareDraft(current[item.id]?.draft ?? "", mode),
    );
    if (!options?.equalizeZero && units.every((unit) => unit <= 0)) {
      return current;
    }
    const normalized = normalizeShares(units);
    if (normalized.length !== selected.length) return current;
    const next = { ...current };
    selected.forEach((item, index) => {
      next[item.id] = {
        ...next[item.id],
        draft: formatShareDraft(normalized[index], mode),
      };
    });
    return next;
  }

  function toggleOperation(id: string, enabled: boolean) {
    setEntries((current) => {
      const entry = current[id];
      if (!entry) return current;
      if (enabled) {
        const selected = cases.filter(
          (item) =>
            current[item.id]?.enabled &&
            caseRisk(item) !== "destructive",
        );
        if (selected.length >= MAX_LOAD_OPERATIONS) {
          return current;
        }
      }
      const units = parseShareDraft(entry.draft, mode);
      const next = {
        ...current,
        [id]: {
          ...entry,
          enabled,
          draft:
            enabled && units <= 0
              ? formatShareDraft(1, mode)
              : entry.draft,
        },
      };
      return mode === "percent" ? rebalance(next) : next;
    });
    setPreviewVersion((current) => current + 1);
  }

  function setAllOperations(enabled: boolean) {
    setEntries((current) => {
      if (!enabled) {
        return Object.fromEntries(
          cases.map((item) => [
            item.id,
            { ...current[item.id], enabled: false },
          ]),
        );
      }
      const eligible = cases.filter(
        (item) =>
          item.enabled && caseRisk(item) !== "destructive",
      );
      const selectedIds = new Set(
        cases
          .filter((item) => current[item.id]?.enabled)
          .map((item) => item.id),
      );
      for (const item of eligible) {
        if (selectedIds.size >= MAX_LOAD_OPERATIONS) break;
        selectedIds.add(item.id);
      }
      const next = Object.fromEntries(
        cases.map((item) => {
          const allowed = eligible.some(
            (candidate) => candidate.id === item.id,
          );
          const selected = allowed && selectedIds.has(item.id);
          const entry = current[item.id];
          const units = parseShareDraft(entry?.draft ?? "", mode);
          return [
            item.id,
            {
              enabled: selected,
              draft:
                selected && units <= 0
                  ? formatShareDraft(1, mode)
                  : entry?.draft ?? formatShareDraft(0, mode),
            },
          ];
        }),
      );
      return mode === "percent" ? rebalance(next) : next;
    });
    setPreviewVersion((current) => current + 1);
  }

  function equalize() {
    setEntries((current) => {
      const selected = cases.filter(
        (item) =>
          current[item.id]?.enabled &&
          caseRisk(item) !== "destructive",
      );
      if (selected.length === 0) return current;
      const shares =
        mode === "percent"
          ? equalShares(selected.length)
          : selected.map(() => 1);
      const next = { ...current };
      selected.forEach((item, index) => {
        next[item.id] = {
          ...next[item.id],
          draft: formatShareDraft(shares[index], mode),
        };
      });
      return next;
    });
  }

  function normalize() {
    setEntries((current) =>
      rebalance(current, { equalizeZero: true }),
    );
  }

  function changeMode(nextMode: ShareInputMode) {
    if (nextMode === mode) return;
    if (nextMode === "weight") {
      if (
        selectedCases.length > 0 &&
        !(
          selectedUnits.every(
            (unit) => unit >= 1 && unit <= SHARE_UNITS,
          ) && totalUnits === SHARE_UNITS
        )
      ) {
        return;
      }
      setEntries((current) =>
        Object.fromEntries(
          cases.map((item) => {
            const units = parseShareDraft(
              current[item.id]?.draft ?? "",
              "percent",
            );
            return [
              item.id,
              {
                ...current[item.id],
                draft: formatShareDraft(units, "weight"),
              },
            ];
          }),
        ),
      );
      setMode("weight");
      return;
    }

    if (
      selectedCases.length > 0 &&
      (totalUnits <= 0 ||
        selectedUnits.some((unit) => unit < 0))
    ) {
      return;
    }
    const hasZero = selectedUnits.some((unit) => unit === 0);
    const hasTiny =
      totalUnits > 0 &&
      selectedUnits.some(
        (unit) => unit > 0 && unit / totalUnits < 0.001,
      );
    if (hasZero || hasTiny) {
      const message =
        hasZero && hasTiny
          ? "有已选用例权重为 0 或低于 0.1%，切换到百分比会把它们提升到 0.1%。继续？"
          : hasZero
            ? "有已选用例权重为 0，切换到百分比会把它提升到 0.1%。继续？"
            : "有操作的占比低于 0.1%，切换到百分比会把它提升到 0.1%。继续？";
      if (!window.confirm(message)) return;
    }

    const normalizedSelected = normalizeShares(selectedUnits);
    const selectedTotal = totalUnits;
    setEntries((current) => {
      const allWeights = cases.map((item) =>
        parseShareDraft(
          current[item.id]?.draft ?? "",
          "weight",
        ),
      );
      const denominator =
        selectedTotal > 0
          ? selectedTotal
          : Math.max(1, sumShares(allWeights));
      const next = { ...current };
      let selectedIndex = 0;
      for (const item of cases) {
        const entry = current[item.id];
        if (!entry) continue;
        const selected =
          entry.enabled && caseRisk(item) !== "destructive";
        let units: number;
        if (selected) {
          units = normalizedSelected[selectedIndex] ?? 0;
          selectedIndex += 1;
        } else {
          const unit = parseShareDraft(entry.draft, "weight");
          units = Math.min(
            SHARE_UNITS,
            Math.max(
              0,
              Math.round((unit / denominator) * SHARE_UNITS),
            ),
          );
        }
        next[item.id] = {
          ...entry,
          draft: formatShareDraft(units, "percent"),
        };
      }
      return next;
    });
    setMode("percent");
  }

  return (
    <Modal
      title="新建压测"
      wide
      onClose={onClose}
      footer={
        <div className="flex items-center gap-2">
          <span
            title={footerStatus}
            className={`mr-auto min-w-0 truncate text-[11px] ${
              localError ? "text-red-600" : "text-slate-400"
            }`}
          >
            {footerStatus}
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
            disabled={!canStart}
            title={startDisabledReason}
            onClick={() => start.mutate()}
          >
            {start.isPending ? "启动中…" : "开始压测"}
          </button>
        </div>
      }
    >
      <div className="grid gap-4 md:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
        <div className="min-w-0 space-y-4">
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
              <Field label="会话模式">
                <select
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={sessionMode}
                  onChange={(event) =>
                    setSessionMode(event.target.value as LoadSessionMode)
                  }
                >
                  <option value="per_vu">每 VU 独立会话</option>
                  <option value="shared">单会话共享</option>
                </select>
              </Field>
            </div>
          </div>

          <details className="rounded border border-slate-200 bg-slate-50/50">
            <summary className="cursor-pointer select-none px-3 py-1.5 text-xs font-medium text-slate-500">
              高级执行参数
              <span className="ml-2 font-normal text-[11px] text-slate-400">
                {advancedSummary}
              </span>
            </summary>
            <div className="grid grid-cols-2 gap-3 border-t border-slate-100 p-3 text-xs">
              <Field label="think time 最小（ms）">
                <input
                  type="number"
                  min={0}
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={thinkMin}
                  onChange={(event) => setThinkMin(Number(event.target.value))}
                />
              </Field>
              <Field label="think time 最大（ms）">
                <input
                  type="number"
                  min={0}
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={thinkMax}
                  onChange={(event) => setThinkMax(Number(event.target.value))}
                />
              </Field>
              <Field label="调用超时（秒）">
                <input
                  type="number"
                  min={1}
                  max={300}
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={timeoutSecs}
                  onChange={(event) =>
                    setTimeoutSecs(Number(event.target.value))
                  }
                />
              </Field>
              <Field label="错误率中止（0–1，空=关闭）">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={abortErrorRate}
                  onChange={(event) => setAbortErrorRate(event.target.value)}
                  placeholder="0.2"
                />
              </Field>
              <Field label="p99 中止（ms，空=关闭）">
                <input
                  className="w-full rounded border border-slate-200 px-2 py-1.5"
                  value={abortP99}
                  onChange={(event) => setAbortP99(event.target.value)}
                  placeholder="例如 500"
                />
              </Field>
              <p className="self-end text-[11px] text-slate-400">
                think time 与应用参数无关；p99 为空表示不按延迟中止。
              </p>
            </div>
          </details>

          <div>
            <div className="mb-2 flex items-center justify-between text-xs font-medium text-slate-500">
              <span>阶段（每阶段 VU 固定）</span>
              <button
                type="button"
                className="rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50"
                onClick={() =>
                  setStages((current) => [
                    ...current,
                    { duration_secs: 30, target_vus: 10 },
                  ])
                }
              >
                添加阶段
              </button>
            </div>
            <div className="space-y-2">
              {stages.map((stage, index) => (
                <div key={index} className="flex items-center gap-2 text-xs">
                  <span className="w-12 text-slate-400">#{index + 1}</span>
                  <input
                    type="number"
                    min={1}
                    className="w-24 rounded border border-slate-200 px-2 py-1"
                    value={stage.duration_secs}
                    onChange={(event) =>
                      updateStage(index, {
                        duration_secs: Number(event.target.value),
                      })
                    }
                  />
                  <span className="text-slate-400">秒</span>
                  <input
                    type="number"
                    min={1}
                    className="w-24 rounded border border-slate-200 px-2 py-1"
                    value={stage.target_vus}
                    onChange={(event) =>
                      updateStage(index, {
                        target_vus: Number(event.target.value),
                      })
                    }
                  />
                  <span className="text-slate-400">VU</span>
                  <button
                    type="button"
                    className="ml-auto rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50 disabled:opacity-40"
                    disabled={stages.length <= 1}
                    onClick={() =>
                      setStages((current) =>
                        current.filter((_, itemIndex) => itemIndex !== index),
                      )
                    }
                  >
                    删除
                  </button>
                </div>
              ))}
            </div>
            <div className="mt-1 text-[11px] text-slate-400">
              总时长{" "}
              {stages.reduce((sum, stage) => sum + stage.duration_secs, 0)} 秒；
              最大 {Math.max(...stages.map((stage) => stage.target_vus), 0)}{" "}
              VU。
            </div>
            <div className="mt-0.5 text-[11px] text-slate-400">
              本版为闭环固定/阶梯 VU；不采样预热、不做 open-loop。
            </div>
          </div>
        </div>

        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-2 text-xs font-medium text-slate-500">
            <span>操作调用占比（{cases.length} 个用例）</span>
            {mode === "percent" ? (
              selectedCases.length === 0 ? (
                <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[11px] text-slate-500">
                  未选择用例
                </span>
              ) : (
                <span
                  role="status"
                  className={`rounded px-1.5 py-0.5 text-[11px] ${
                    percentTotalValid
                      ? "bg-emerald-50 text-emerald-600"
                      : "bg-red-50 text-red-600"
                  }`}
                >
                  合计 {formatUnitsAsPercent(totalUnits)}
                  {percentTotalValid ? "" : "（应为 100.0%）"}
                </span>
              )
            ) : (
              <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[11px] text-slate-500">
                {selectedCases.length === 0
                  ? "未选择用例"
                  : `相对权重合计 ${totalUnits}`}
              </span>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <div className="inline-flex overflow-hidden rounded border border-slate-200">
              <button
                type="button"
                aria-pressed={mode === "percent"}
                className={`px-2 py-1 text-[11px] ${
                  mode === "percent"
                    ? "bg-slate-800 text-white"
                    : "bg-white text-slate-600 hover:bg-slate-50"
                } disabled:opacity-40`}
                disabled={percentModeDisabled}
                onClick={() => changeMode("percent")}
              >
                百分比
              </button>
              <button
                type="button"
                aria-pressed={mode === "weight"}
                className={`px-2 py-1 text-[11px] ${
                  mode === "weight"
                    ? "bg-slate-800 text-white"
                    : "bg-white text-slate-600 hover:bg-slate-50"
                } disabled:opacity-40`}
                disabled={weightModeDisabled}
                onClick={() => changeMode("weight")}
              >
                权重
              </button>
            </div>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50 disabled:opacity-40"
              disabled={selectedCases.length === 0}
              onClick={equalize}
            >
              均分
            </button>
            {mode === "percent" ? (
              <button
                type="button"
                className="rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50 disabled:opacity-40"
                disabled={selectedCases.length === 0}
                onClick={normalize}
              >
                归一化
              </button>
            ) : null}
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50"
              onClick={() => setAllOperations(true)}
            >
              全选
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50"
              onClick={() => setAllOperations(false)}
            >
              全不选
            </button>
            <button
              type="button"
              className="ml-auto rounded border border-slate-200 px-2 py-1 text-[11px] hover:bg-slate-50 disabled:opacity-50"
              disabled={previewQuery.isFetching}
              onClick={() => setPreviewVersion((current) => current + 1)}
            >
              {previewQuery.isFetching ? "评估中…" : "重新评估风险"}
            </button>
          </div>
          <div className="flex max-h-[min(36vh,440px)] flex-col overflow-hidden rounded border border-slate-200">
            <div className="grid grid-cols-[20px_minmax(0,1fr)_44px_92px] items-center gap-2 border-b border-slate-100 bg-slate-50 px-3 py-1 text-[10px] text-slate-400">
              <span />
              <span>用例</span>
              <span className="text-center">风险</span>
              <span className="text-right">
                {mode === "percent" ? "占比" : "权重"}
              </span>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-1">
              {cases.map((item) => {
                const entry = entries[item.id] ?? {
                  enabled: false,
                  draft: "0.0",
                };
                const effectiveRisk = caseRisk(item);
                const destructive = effectiveRisk === "destructive";
                const atCap = selectedCases.length >= MAX_LOAD_OPERATIONS;
                const checkboxDisabled =
                  destructive || (!entry.enabled && atCap);
                const inputDisabled = !entry.enabled || destructive;
                return (
                  <div
                    key={item.id}
                    className="grid grid-cols-[20px_minmax(0,1fr)_44px_92px] items-center gap-2 rounded px-2 py-1 text-xs hover:bg-slate-50"
                  >
                    <input
                      type="checkbox"
                      aria-label={`选择用例 ${item.name}`}
                      checked={entry.enabled && !destructive}
                      disabled={checkboxDisabled}
                      title={
                        destructive
                          ? "破坏性操作禁止压测"
                          : checkboxDisabled
                            ? `最多选择 ${MAX_LOAD_OPERATIONS} 个用例`
                            : undefined
                      }
                      onChange={(event) =>
                        toggleOperation(item.id, event.target.checked)
                      }
                    />
                    <div className="min-w-0">
                      <div className="truncate text-slate-700">{item.name}</div>
                      <div className="truncate text-[10px] text-slate-400">
                        {item.method} · {item.target}
                      </div>
                    </div>
                    <span
                      className={`justify-self-center whitespace-nowrap rounded px-1.5 py-0.5 text-[10px] ${RISK_CLASS[effectiveRisk]}`}
                    >
                      {RISK_LABEL[effectiveRisk]}
                    </span>
                    {mode === "percent" ? (
                      <div className="flex items-center justify-end gap-1">
                        <input
                          type="text"
                          inputMode="decimal"
                          className="w-16 rounded border border-slate-200 px-1 py-0.5 text-right tabular-nums disabled:bg-slate-50 disabled:text-slate-400"
                          value={entry.draft}
                          disabled={inputDisabled}
                          aria-label={`用例 ${item.name} 的占比`}
                          onChange={(event) =>
                            updateDraft(item.id, event.target.value)
                          }
                          onBlur={() => canonicalizeDraft(item.id)}
                        />
                        <span className="text-slate-400">%</span>
                      </div>
                    ) : (
                      <div className="flex flex-col items-end gap-0.5">
                        <input
                          type="text"
                          inputMode="numeric"
                          className="w-16 rounded border border-slate-200 px-1 py-0.5 text-right tabular-nums disabled:bg-slate-50 disabled:text-slate-400"
                          value={entry.draft}
                          disabled={inputDisabled}
                          aria-label={`用例 ${item.name} 的权重`}
                          onChange={(event) =>
                            updateDraft(item.id, event.target.value)
                          }
                          onBlur={() => canonicalizeDraft(item.id)}
                        />
                        <span className="text-[10px] leading-none text-slate-400">
                          {entry.enabled && !destructive
                            ? `≈${formatPercentValue(
                                weightPercentByCase.get(item.id) ?? 0,
                              )}`
                            : "-"}
                        </span>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
          <div
            className="text-[11px] text-slate-400"
            title="新增项默认 0.1%，重新勾选恢复原输入并参与归一化"
          >
            {mode === "percent"
              ? "百分比 = 每次调用被选中的概率，不是 VU 比例；并发由阶段目标 VU 决定。"
              : "权重是相对值，1:3 = 25%:75%；不会改变阶段目标 VU。"}
          </div>
          {cases.filter(
            (item) => item.enabled && caseRisk(item) !== "destructive",
          ).length > MAX_LOAD_OPERATIONS ? (
            <div className="mt-1 text-[11px] text-amber-600">
              可用用例超过 {MAX_LOAD_OPERATIONS} 个，最多同时选择{" "}
              {MAX_LOAD_OPERATIONS} 个；默认选择前 {MAX_LOAD_OPERATIONS} 个。
            </div>
          ) : null}
        </div>
      </div>

      <div className="mt-4 space-y-2 rounded border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800">
        <div className="font-medium">风险确认</div>
        <div>
          只读操作可直接运行；write/unknown 需要授权；destructive 禁止压测。
          生产标签服务器需要额外确认。
        </div>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={allowWrite}
            onChange={(event) => {
              setAllowWrite(event.target.checked);
              setPreviewVersion((current) => current + 1);
            }}
          />
          允许写操作（write）
        </label>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={allowUnknown}
            onChange={(event) => {
              setAllowUnknown(event.target.checked);
              setPreviewVersion((current) => current + 1);
            }}
          />
          允许未知风险（unknown）
        </label>
        {preview?.production ? (
          <label className="flex items-center gap-2 font-medium text-red-600">
            <input
              type="checkbox"
              checked={confirmProduction}
              onChange={(event) => setConfirmProduction(event.target.checked)}
            />
            我确认这是生产服务器，并已获得压测授权
          </label>
        ) : null}
        {preview?.warnings.map((warning) => (
          <div key={warning}>{warning}</div>
        ))}
        {preview?.operations
          .filter((operation) => !operation.allowed)
          .slice(0, 5)
          .map((operation) => (
            <div key={operation.case_id} className="text-red-600">
              {operation.name}：{operation.reason ?? "不可运行"}
            </div>
          ))}
        {previewError ? (
          <div className="text-red-600">
            风险评估失败：{previewError}，请重试
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
