import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSearchParams } from "react-router-dom";
import { save } from "@tauri-apps/plugin-dialog";
import { EmptyState } from "../components/EmptyState";
import { Field, Modal } from "../components/testing/SuiteDialog";
import { RunCaseDetail } from "../components/testing/RunCaseDetail";
import {
  compareTestRuns,
  exportTestReport,
  getTestRun,
  getTestRunCase,
  listTestRunCases,
  listTestRuns,
  rerunFailedTestCases,
  setTestBaseline,
} from "../api/tests";
import { toast } from "../stores/toast";
import {
  CASE_STATUS_CLASS,
  CASE_STATUS_FILTERS,
  CASE_STATUS_LABEL,
  FAILURE_KIND_LABEL,
  RUN_STATUS_CLASS,
  RUN_STATUS_LABEL,
  skipReasonLabel,
} from "../components/testing/labels";
import type {
  RunComparison,
  TestRunCaseStatus,
  TestRunStatus,
} from "../types/testing";

export function Reports() {
  const queryClient = useQueryClient();
  const [runStatus, setRunStatus] = useState<"" | TestRunStatus>("");
  const [searchParams, setSearchParams] = useSearchParams();
  // 当前运行由 URL 的 run 参数决定，便于从测试页「查看报告」直接定位。
  const selectedRunId = searchParams.get("run") ?? "";
  const [caseFilter, setCaseFilter] = useState<{
    runId: string;
    status: "" | TestRunCaseStatus;
  }>({ runId: "", status: "" });
  const [selectedCaseId, setSelectedCaseId] = useState<string | null>(null);
  const [compareOpen, setCompareOpen] = useState(false);
  const [baseRunId, setBaseRunId] = useState("");
  const [comparison, setComparison] = useState<RunComparison | null>(null);
  const [includeSensitive, setIncludeSensitive] = useState(false);

  const runs = useQuery({
    queryKey: ["test-runs", runStatus],
    queryFn: () =>
      listTestRuns(
        { status: runStatus || null },
        0,
        100,
      ),
    refetchInterval: 5000,
  });
  const runList = runs.data?.items ?? [];
  const activeRun =
    runList.find((run) => run.id === selectedRunId) ?? runList[0];
  const effectiveCaseStatus =
    caseFilter.runId === activeRun?.id ? caseFilter.status : "";

  const detail = useQuery({
    queryKey: ["test-run", activeRun?.id],
    queryFn: () => getTestRun(activeRun?.id ?? ""),
    enabled: Boolean(activeRun?.id),
  });

  const cases = useQuery({
    queryKey: ["test-run-cases", activeRun?.id, effectiveCaseStatus],
    queryFn: () =>
      listTestRunCases(
        activeRun?.id ?? "",
        { status: effectiveCaseStatus || null },
        0,
        500,
      ),
    enabled: Boolean(activeRun?.id),
  });

  const caseDetail = useQuery({
    queryKey: ["test-run-case", selectedCaseId],
    queryFn: () => getTestRunCase(selectedCaseId ?? ""),
    enabled: Boolean(selectedCaseId),
  });

  const baseline = useMutation({
    mutationFn: (runId: string) => setTestBaseline(runId),
    onSuccess: () => {
      toast.success("已设为基线");
      void queryClient.invalidateQueries({ queryKey: ["test-runs"] });
    },
    onError: (error) => toast.fromError(error, "设置基线失败"),
  });

  const rerun = useMutation({
    mutationFn: (runId: string) =>
      rerunFailedTestCases(runId, {
        stop_on_failure: false,
        retry_count: 0,
        allow_write: false,
        allow_destructive: false,
        allow_unknown: false,
        post_case_health: false,
        seed: 20261021,
      }),
    onSuccess: () => {
      toast.success("已启动失败用例重跑");
      void queryClient.invalidateQueries({ queryKey: ["test-runs"] });
    },
    onError: (error) => toast.fromError(error, "重跑失败"),
  });

  const compare = useMutation({
    mutationFn: () => compareTestRuns(baseRunId, activeRun?.id ?? ""),
    onSuccess: (result) => setComparison(result),
    onError: (error) => toast.fromError(error, "对比失败"),
  });

  async function exportReport(format: "html" | "json" | "junit" | "md") {
    if (!activeRun) return;
    const extension = format === "junit" ? "xml" : format === "md" ? "md" : format;
    const path = await save({
      title: "导出测试报告",
      defaultPath: `mcptester-report-${activeRun.id.slice(0, 8)}.${extension}`,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!path) return;
    try {
      await exportTestReport(
        activeRun.id,
        format,
        path,
        false,
        includeSensitive,
      );
      toast.success("报告已导出", { message: path });
    } catch (error) {
      toast.fromError(error, "导出报告失败");
    }
  }

  const totals = detail.data?.run.summary;
  const coverage = detail.data?.coverage ?? totals?.coverage;
  const failedCount = (totals?.failed ?? 0) + (totals?.error ?? 0);
  const caseItems = cases.data?.items ?? [];
  const caseTotal = cases.data?.total ?? 0;

  return (
    <section className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-xl font-semibold text-slate-800">测试报告</h2>
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={runStatus}
          onChange={(event) => {
            setRunStatus(event.target.value as "" | TestRunStatus);
            setSearchParams({}, { replace: true });
            setCaseFilter({ runId: "", status: "" });
          }}
        >
          <option value="">全部状态</option>
          <option value="completed">完成</option>
          <option value="failed">失败</option>
          <option value="canceled">取消</option>
          <option value="running">运行中</option>
          <option value="interrupted">中断</option>
        </select>
        <label className="ml-auto flex items-center gap-1 text-[11px] text-slate-500">
          <input
            type="checkbox"
            checked={includeSensitive}
            onChange={(event) => setIncludeSensitive(event.target.checked)}
          />
          导出包含敏感原文
        </label>
        {activeRun ? (
          <>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
              onClick={() => baseline.mutate(activeRun.id)}
            >
              设为基线
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
              disabled={failedCount === 0 || rerun.isPending}
              onClick={() => rerun.mutate(activeRun.id)}
            >
              重跑失败（{failedCount}）
            </button>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
              onClick={() => {
                setBaseRunId("");
                setComparison(null);
                setCompareOpen(true);
              }}
            >
              对比基线
            </button>
            <div className="flex items-center gap-1">
              {(["html", "json", "junit", "md"] as const).map((format) => (
                <button
                  key={format}
                  type="button"
                  className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
                  onClick={() => void exportReport(format)}
                >
                  {format === "junit" ? "JUnit" : format.toUpperCase()}
                </button>
              ))}
            </div>
          </>
        ) : null}
      </div>

      {runs.isLoading ? (
        <div className="p-3 text-xs text-slate-400">加载运行记录…</div>
      ) : runList.length === 0 ? (
        <EmptyState
          title="还没有测试运行"
          description="在“测试”页运行套件后，这里会出现报告。"
        />
      ) : (
        <div className="flex min-h-0 flex-1 gap-3">
          <aside className="w-80 shrink-0 overflow-y-auto rounded-lg border border-slate-200 bg-white">
            {runList.map((run) => (
              <button
                key={run.id}
                type="button"
                className={`block w-full border-b border-slate-100 px-3 py-2 text-left text-xs last:border-b-0 ${
                  activeRun?.id === run.id ? "bg-slate-100" : "hover:bg-slate-50"
                }`}
                onClick={() => {
                  setSearchParams({ run: run.id }, { replace: true });
                  setSelectedCaseId(null);
                  setCaseFilter({ runId: "", status: "" });
                }}
              >
                <div className="flex items-center gap-2">
                  <span className={`font-medium ${RUN_STATUS_CLASS[run.status]}`}>
                    {RUN_STATUS_LABEL[run.status] ?? run.status}
                  </span>
                  <span className="truncate text-slate-800">
                    {run.suite_name}
                  </span>
                  {run.baseline ? (
                    <span className="ml-auto rounded bg-emerald-100 px-1 text-[10px] text-emerald-700">
                      基线
                    </span>
                  ) : null}
                </div>
                <div className="mt-0.5 text-slate-400">
                  {run.server_name} · {run.total} 条 · 通过{" "}
                  {(run.effective_pass_rate * 100).toFixed(1)}% ·{" "}
                  {new Date(run.started_at).toLocaleString()}
                </div>
              </button>
            ))}
          </aside>

          <div className="min-w-0 flex-1 overflow-y-auto">
            {!activeRun || detail.isLoading ? (
              <div className="text-xs text-slate-400">加载报告…</div>
            ) : detail.data && totals ? (
              <div className="space-y-3">
                <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
                  <SummaryCard
                    label="全部"
                    value={totals.total}
                    active={!effectiveCaseStatus}
                    ariaLabel={`显示全部用例（${totals.total}）`}
                    onClick={() => setCaseFilter({ runId: "", status: "" })}
                  />
                  {CASE_STATUS_FILTERS.map(({ value, label }) => (
                    <SummaryCard
                      key={value}
                      label={label}
                      value={totals[value]}
                      className={CASE_STATUS_CLASS[value]}
                      active={effectiveCaseStatus === value}
                      ariaLabel={`筛选：${label}（${totals[value]}）`}
                      onClick={() =>
                        setCaseFilter({
                          runId: activeRun?.id ?? "",
                          status: effectiveCaseStatus === value ? "" : value,
                        })
                      }
                    />
                  ))}
                  <SummaryCard
                    label="有效通过率"
                    value={`${(totals.effective_pass_rate * 100).toFixed(1)}%`}
                    className="text-emerald-600"
                  />
                </div>

                <div className="rounded-lg border border-slate-200 bg-white p-3">
                  <h3 className="mb-2 text-xs font-medium text-slate-500">
                    覆盖率
                  </h3>
                  <div className="grid grid-cols-2 gap-2 text-xs md:grid-cols-4">
                    <CoverageItem
                      label="Tools"
                      covered={coverage?.tools_covered ?? 0}
                      total={coverage?.tools_total ?? 0}
                    />
                    <CoverageItem
                      label="Resources"
                      covered={coverage?.resources_covered ?? 0}
                      total={coverage?.resources_total ?? 0}
                    />
                    <CoverageItem
                      label="Prompts"
                      covered={coverage?.prompts_covered ?? 0}
                      total={coverage?.prompts_total ?? 0}
                    />
                    <CoverageItem
                      label="必填参数"
                      covered={coverage?.required_params_covered ?? 0}
                      total={coverage?.required_params_total ?? 0}
                    />
                  </div>
                </div>

                <div className="rounded-lg border border-slate-200 bg-white">
                  <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 p-2">
                    <span className="text-xs font-medium text-slate-600">
                      用例结果
                    </span>
                    {effectiveCaseStatus ? (
                      <span className="text-[11px] text-slate-500">
                        筛选：{CASE_STATUS_LABEL[effectiveCaseStatus]}
                      </span>
                    ) : null}
                    <span className="text-[11px] text-slate-400">
                      {caseTotal}/{totals.total} 条
                    </span>
                    <div className="ml-auto flex items-center gap-2">
                      {effectiveCaseStatus ? (
                        <button
                          type="button"
                          className="rounded border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                          onClick={() =>
                            setCaseFilter({ runId: "", status: "" })
                          }
                        >
                          清除筛选
                        </button>
                      ) : null}
                    </div>
                  </div>
                  <table className="w-full text-left text-xs">
                    <thead className="bg-slate-50 text-slate-500">
                      <tr>
                        <th className="px-2 py-2">状态</th>
                        <th className="px-2 py-2">用例</th>
                        <th className="px-2 py-2">方法/目标</th>
                        <th className="px-2 py-2">耗时</th>
                        <th className="px-2 py-2">失败分类</th>
                      </tr>
                    </thead>
                    <tbody>
                      {cases.isLoading ? (
                        <tr>
                          <td
                            colSpan={5}
                            className="px-2 py-6 text-center text-slate-400"
                          >
                            加载用例结果…
                          </td>
                        </tr>
                      ) : cases.isError ? (
                        <tr>
                          <td colSpan={5} className="px-2 py-6 text-center">
                            <span className="text-red-500">用例结果加载失败。</span>
                            <button
                              type="button"
                              className="ml-2 rounded border border-slate-200 px-2 py-0.5 text-slate-600 hover:bg-slate-50"
                              onClick={() => void cases.refetch()}
                            >
                              重试
                            </button>
                          </td>
                        </tr>
                      ) : caseItems.length === 0 ? (
                        <tr>
                          <td
                            colSpan={5}
                            className="px-2 py-6 text-center text-slate-400"
                          >
                            {totals.total === 0
                              ? "本次运行没有用例。"
                              : "没有符合筛选的用例。"}
                            {effectiveCaseStatus ? (
                              <button
                                type="button"
                                className="ml-2 rounded border border-slate-200 px-2 py-0.5 text-slate-600 hover:bg-slate-50"
                                onClick={() =>
                                  setCaseFilter({ runId: "", status: "" })
                                }
                              >
                                清除筛选
                              </button>
                            ) : null}
                          </td>
                        </tr>
                      ) : (
                        caseItems.map((item) => (
                          <tr
                            key={item.id}
                            className="cursor-pointer border-t border-slate-100 hover:bg-slate-50"
                            onClick={() => setSelectedCaseId(item.id)}
                          >
                            <td
                              className={`px-2 py-2 ${CASE_STATUS_CLASS[item.status]}`}
                            >
                              {CASE_STATUS_LABEL[item.status] ?? item.status}
                            </td>
                            <td className="max-w-64 px-2 py-2">
                              <div className="truncate text-slate-800">
                                {item.case_name}
                              </div>
                              {item.status === "skipped" ? (
                                <div className="truncate text-[10px] text-slate-400">
                                  跳过原因：{skipReasonLabel(item.skip_reason)}
                                </div>
                              ) : item.message ? (
                                <div className="truncate text-[10px] text-slate-400">
                                  {item.message}
                                </div>
                              ) : null}
                            </td>
                            <td className="px-2 py-2 text-slate-500">
                              {item.method} · {item.target}
                            </td>
                            <td className="px-2 py-2 text-slate-500">
                              {item.duration_ms}ms
                            </td>
                            <td className="px-2 py-2 text-slate-500">
                              {item.failure_kind
                                ? (FAILURE_KIND_LABEL[item.failure_kind] ??
                                  item.failure_kind)
                                : "-"}
                            </td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>

                <div className="rounded-lg border border-slate-200 bg-white p-3 text-[11px] text-slate-500">
                  运行清单：{detail.data.run.manifest.app_version} ·{" "}
                  {detail.data.run.manifest.protocol_version} ·{" "}
                  {detail.data.run.manifest.transport} · 能力指纹{" "}
                  <code>{detail.data.run.manifest.capability_hash}</code> · 种子{" "}
                  {detail.data.run.manifest.seed}
                  {detail.data.run.manifest.capability_stale
                    ? " · 能力快照可能过期"
                    : ""}
                </div>
              </div>
            ) : (
              <div className="text-sm text-red-500">报告加载失败</div>
            )}
          </div>
        </div>
      )}

      {selectedCaseId ? (
        <Modal
          title="用例详情"
          xwide
          onClose={() => setSelectedCaseId(null)}
        >
          {caseDetail.isLoading ? (
            <div className="text-xs text-slate-400">加载中…</div>
          ) : caseDetail.data ? (
            <RunCaseDetail detail={caseDetail.data} />
          ) : (
            <div className="text-xs text-red-500">用例详情加载失败</div>
          )}
        </Modal>
      ) : null}

      {compareOpen ? (
        <Modal
          title="与基线对比"
          onClose={() => setCompareOpen(false)}
        >
          <div className="space-y-3 text-sm">
            <Field label="基线运行">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={baseRunId}
                onChange={(event) => setBaseRunId(event.target.value)}
              >
                <option value="">请选择</option>
                {runList
                  .filter((run) => run.id !== activeRun?.id)
                  .map((run) => (
                    <option key={run.id} value={run.id}>
                      {new Date(run.started_at).toLocaleString()} ·{" "}
                      {run.suite_name} · {run.status}
                    </option>
                  ))}
              </select>
            </Field>
            <button
              type="button"
              className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
              disabled={!baseRunId || compare.isPending}
              onClick={() => compare.mutate()}
            >
              {compare.isPending ? "对比中…" : "开始对比"}
            </button>
            {comparison ? <ComparisonView value={comparison} /> : null}
          </div>
        </Modal>
      ) : null}
    </section>
  );
}

function SummaryCard({
  label,
  value,
  className,
  active,
  onClick,
  ariaLabel,
}: {
  label: string;
  value: number | string;
  className?: string;
  active?: boolean;
  onClick?: () => void;
  ariaLabel?: string;
}) {
  const body = (
    <>
      <div className="text-[11px] text-slate-400">{label}</div>
      <div className={`text-lg font-semibold ${className ?? "text-slate-800"}`}>
        {value}
      </div>
    </>
  );
  const base = `rounded-lg border p-2 ${
    active
      ? "border-slate-800 bg-slate-50 ring-1 ring-slate-800"
      : "border-slate-200 bg-white"
  }`;
  if (!onClick) {
    return <div className={base}>{body}</div>;
  }
  return (
    <button
      type="button"
      className={`${base} w-full text-left hover:border-slate-300 focus:outline-none focus-visible:ring-2 focus-visible:ring-slate-500`}
      aria-label={ariaLabel}
      aria-pressed={Boolean(active)}
      onClick={onClick}
    >
      {body}
    </button>
  );
}

function CoverageItem({
  label,
  covered,
  total,
}: {
  label: string;
  covered: number;
  total: number;
}) {
  const percent = total > 0 ? Math.round((covered / total) * 100) : 0;
  return (
    <div>
      <div className="flex justify-between text-slate-500">
        <span>{label}</span>
        <span>
          {covered}/{total} · {percent}%
        </span>
      </div>
      <div className="mt-1 h-1.5 overflow-hidden rounded bg-slate-100">
        <div className="h-full bg-sky-500" style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

function ComparisonView({ value }: { value: RunComparison }) {
  const rows: { label: string; items: typeof value.new_failures }[] = [
    { label: "新增失败", items: value.new_failures },
    { label: "修复", items: value.fixed },
    { label: "持续失败", items: value.still_failing },
    { label: "警告变化", items: value.warning_changes },
    { label: "错误", items: value.error_cases },
    { label: "新增用例", items: value.new_cases },
    { label: "删除用例", items: value.removed_cases },
  ];
  return (
    <div className="space-y-2 text-xs">
      <div className="grid grid-cols-3 gap-2">
        <SummaryCard label="新增失败" value={value.new_failures.length} />
        <SummaryCard label="修复" value={value.fixed.length} />
        <SummaryCard label="持续失败" value={value.still_failing.length} />
      </div>
      {rows.map((row) =>
        row.items.length > 0 ? (
          <details key={row.label} open>
            <summary className="cursor-pointer text-slate-600">
              {row.label}（{row.items.length}）
            </summary>
            <ul className="mt-1 list-disc pl-5 text-slate-500">
              {row.items.map((item) => (
                <li key={item.key}>{item.name}</li>
              ))}
            </ul>
          </details>
        ) : null,
      )}
      <div className="text-slate-400">
        有效通过率变化 {(value.effective_pass_rate_delta * 100).toFixed(1)}% ·
        耗时 {value.duration_delta_ms}ms
        {value.contract_changed ? " · 接口定义已变化" : ""}
      </div>
    </div>
  );
}
