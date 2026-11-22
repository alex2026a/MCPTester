import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { exportLoadRun } from "../../api/loadtest";
import { toast } from "../../stores/toast";
import type {
  LoadOperationSummary,
  LoadRun,
  LoadRunStatus,
} from "../../types/loadtest";
import {
  formatPercentValue,
  formatUnitsAsPercent,
  normalizeShares,
  sumShares,
} from "../../lib/loadAllocation";
import { SeriesChart } from "./SeriesChart";

const STATUS_LABEL: Record<LoadRunStatus, string> = {
  running: "运行中",
  completed: "完成",
  failed: "失败",
  canceled: "取消",
  interrupted: "中断",
  aborted: "自动中止",
};

interface ConfiguredOperationRow {
  caseId: string;
  name: string;
  weight: number;
  summary?: LoadOperationSummary;
}

interface OperationRow extends ConfiguredOperationRow {
  configured: boolean;
}

function formatDuration(ms: number): string {
  const seconds = Math.abs(Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

/** 压测结果详情。 */
export function RunResult({ run }: { run: LoadRun }) {
  const [exporting, setExporting] = useState(false);
  const summary = run.summary;
  const hasSamples = summary.eligible > 0;
  const distributions = [
    ["成功", summary.ok, "text-emerald-600"],
    ["工具错误", summary.tool_error, "text-amber-600"],
    ["RPC 错误", summary.rpc_error, "text-red-600"],
    ["超时", summary.timeout, "text-red-600"],
    ["传输错误", summary.transport_error, "text-red-600"],
    ["认证错误", summary.auth_error, "text-red-600"],
    ["取消", summary.canceled, "text-slate-500"],
    ["输入请求", summary.input_required, "text-amber-600"],
    ["其他", summary.other_error, "text-red-600"],
  ] as const;

  const summaryOperations = summary.operations ?? [];
  const summaryByCase = new Map(
    summaryOperations.map((operation) => [operation.case_id, operation]),
  );
  const configured: ConfiguredOperationRow[] =
    (run.operations ?? []).length > 0
      ? run.operations.map((operation) => ({
          caseId: operation.case_id,
          name: operation.name,
          weight: operation.weight,
          summary: summaryByCase.get(operation.case_id),
        }))
      : (run.profile.operations ?? []).map((operation) => {
          const item = summaryByCase.get(operation.case_id);
          return {
            caseId: operation.case_id,
            name: item?.name ?? operation.case_id,
            weight: operation.weight,
            summary: item,
          };
        });
  const configuredWeights = configured.map((row) => row.weight);
  const configuredUnits =
    sumShares(configuredWeights) > 0
      ? normalizeShares(configuredWeights)
      : [];
  const configuredUnitByCase = new Map<string, number>();
  if (configuredUnits.length > 0) {
    configured.forEach((row, index) =>
      configuredUnitByCase.set(row.caseId, configuredUnits[index] ?? 0),
    );
  }
  const configuredIds = new Set(configured.map((row) => row.caseId));
  const extras: OperationRow[] = summaryOperations
    .filter((operation) => !configuredIds.has(operation.case_id))
    .map((operation) => ({
      caseId: operation.case_id,
      name: operation.name,
      weight: 0,
      summary: operation,
      configured: false,
    }));
  const rows: OperationRow[] = [
    ...configured.map((row) => ({ ...row, configured: true })),
    ...extras,
  ];

  async function exportJson() {
    const path = await save({
      title: "导出压测 JSON",
      defaultPath: `mcptester-load-${run.id.slice(0, 8)}.json`,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    setExporting(true);
    try {
      await exportLoadRun(run.id, path);
      toast.success("压测 JSON 已导出", { message: path });
    } catch (error) {
      toast.fromError(error, "导出压测 JSON 失败");
    } finally {
      setExporting(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-slate-200 bg-white p-3">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-sm font-semibold text-slate-800">
            {run.profile_name}
          </h3>
          <span className="rounded bg-slate-100 px-2 py-0.5 text-[11px] text-slate-600">
            {STATUS_LABEL[run.status]}
          </span>
          <span className="text-xs text-slate-400">
            {run.suite_name} · {run.server_name}
          </span>
          {run.stop_reason ? (
            <span className="text-[11px] text-red-500">
              停止原因：{run.stop_reason}
            </span>
          ) : null}
          <button
            type="button"
            className="ml-auto rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={exporting}
            onClick={() => void exportJson()}
          >
            {exporting ? "导出中…" : "导出 JSON"}
          </button>
        </div>
        <div className="mt-3 grid grid-cols-2 gap-2 md:grid-cols-6">
          <Metric label="总调用" value={String(summary.total)} />
          <Metric
            label="成功率"
            value={
              hasSamples
                ? formatPercentValue(summary.success_rate * 100)
                : "-"
            }
          />
          <Metric label="RPS" value={summary.rps.toFixed(2)} />
          <Metric
            label="p95"
            value={hasSamples ? `${summary.p95_ms}ms` : "-"}
          />
          <Metric
            label="p99"
            value={hasSamples ? `${summary.p99_ms}ms` : "-"}
          />
          <Metric label="时长" value={formatDuration(summary.duration_ms)} />
        </div>
        <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[11px]">
          {distributions.map(([label, value, tone]) => (
            <span key={label} className={tone}>
              {label} {value}
            </span>
          ))}
          {!hasSamples ? (
            <span className="text-slate-400">
              无有效调用样本（全部取消或未调用）
            </span>
          ) : null}
        </div>
      </div>

      <div className="grid gap-2 md:grid-cols-3">
        <SeriesChart
          series={[
            {
              label: "RPS",
              color: "#0284c7",
              values: run.series.map((point) => point.rps),
            },
          ]}
          unit="req/s"
        />
        <SeriesChart
          series={[
            {
              label: "p95",
              color: "#7c3aed",
              values: run.series.map((point) => point.p95_ms),
            },
            {
              label: "p99",
              color: "#db2777",
              values: run.series.map((point) => point.p99_ms),
            },
          ]}
          unit="ms"
        />
        <SeriesChart
          series={[
            {
              label: "错误率",
              color: "#dc2626",
              values: run.series.map((point) => point.error_rate * 100),
            },
          ]}
          unit="%"
        />
      </div>

      <div className="rounded-lg border border-slate-200 bg-white p-3">
        <div className="mb-2 text-xs font-medium text-slate-500">按操作拆分</div>
        <div className="mb-1 text-[11px] text-slate-400">
          占比均按调用次数计算；配置占比按权重归一化，实际占比按有效调用数
          （不含取消）。混合取消时延迟仍包含取消样本。
        </div>
        {rows.length === 0 ? (
          <div className="py-2 text-xs text-slate-400">暂无按操作数据</div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead className="text-[11px] text-slate-400">
                <tr>
                  <th className="py-1 pr-3">用例</th>
                  <th className="py-1 pr-3">配置占比</th>
                  <th className="py-1 pr-3">总数 / 实际占比</th>
                  <th className="py-1 pr-3">成功</th>
                  <th className="py-1 pr-3">错误</th>
                  <th className="py-1 pr-3">成功率</th>
                  <th className="py-1 pr-3">avg</th>
                  <th className="py-1 pr-3">p95</th>
                  <th className="py-1">p99</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => {
                  const eligible = row.summary?.eligible ?? 0;
                  const total = row.summary?.total ?? 0;
                  const canceled = row.summary?.canceled ?? 0;
                  const configuredText =
                    row.configured && configuredUnits.length > 0
                      ? formatUnitsAsPercent(
                          configuredUnitByCase.get(row.caseId) ?? 0,
                        )
                      : "-";
                  const actualText = hasSamples
                    ? formatPercentValue((eligible / summary.eligible) * 100)
                    : "-";
                  return (
                    <tr
                      key={row.caseId}
                      className="border-t border-slate-100"
                    >
                      <td className="py-1 pr-3 text-slate-700">{row.name}</td>
                      <td className="py-1 pr-3">
                        <div>{configuredText}</div>
                        <div className="text-[10px] text-slate-400">
                          {row.configured ? `权重 ${row.weight}` : "-"}
                        </div>
                      </td>
                      <td className="py-1 pr-3">
                        <div>{total}</div>
                        <div className="text-[10px] text-slate-400">
                          实际 {actualText}
                          {canceled > 0 ? ` · 取消 ${canceled}` : ""}
                        </div>
                      </td>
                      <td className="py-1 pr-3 text-emerald-600">
                        {row.summary?.ok ?? 0}
                      </td>
                      <td className="py-1 pr-3 text-red-600">
                        {row.summary?.errors ?? 0}
                      </td>
                      <td className="py-1 pr-3">
                        {eligible > 0
                          ? formatPercentValue(
                              (row.summary?.success_rate ?? 0) * 100,
                            )
                          : "-"}
                      </td>
                      <td className="py-1 pr-3">
                        {eligible > 0
                          ? `${(row.summary?.avg_ms ?? 0).toFixed(1)}ms`
                          : "-"}
                      </td>
                      <td className="py-1 pr-3">
                        {eligible > 0
                          ? `${row.summary?.p95_ms ?? 0}ms`
                          : "-"}
                      </td>
                      <td className="py-1">
                        {eligible > 0
                          ? `${row.summary?.p99_ms ?? 0}ms`
                          : "-"}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className="grid gap-2 md:grid-cols-2">
        <div className="rounded-lg border border-slate-200 bg-white p-3">
          <div className="mb-2 text-xs font-medium text-slate-500">
            错误样本（最多 50 条）
          </div>
          <div className="max-h-48 space-y-1 overflow-y-auto text-[11px]">
            {run.errors.length === 0 ? (
              <div className="text-slate-400">无错误样本</div>
            ) : (
              run.errors.map((error, index) => (
                <div key={`${error.at}-${index}`} className="text-red-600">
                  {error.case_name} · {error.status} · {error.duration_ms}ms ·{" "}
                  {error.message}
                </div>
              ))
            )}
          </div>
        </div>
        <div className="rounded-lg border border-slate-200 bg-white p-3 text-xs">
          <div className="mb-2 text-xs font-medium text-slate-500">运行环境</div>
          <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-[11px] text-slate-600">
            <span>应用版本</span>
            <span>{run.manifest.app_version}</span>
            <span>传输</span>
            <span>{run.manifest.transport}</span>
            <span>协议</span>
            <span>{run.manifest.protocol_version}</span>
            <span>生命周期</span>
            <span>{run.manifest.lifecycle}</span>
            <span>能力指纹</span>
            <span className="truncate">{run.manifest.capability_hash}</span>
            <span>开始时间</span>
            <span>{new Date(run.started_at).toLocaleString()}</span>
            <span>结束时间</span>
            <span>
              {run.finished_at
                ? new Date(run.finished_at).toLocaleString()
                : "-"}
            </span>
          </div>
          {run.manifest.capability_stale ? (
            <div className="mt-2 text-amber-600">
              能力快照为缓存（stale），write/unknown 风险已按 unknown 处理。
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div
      data-testid={`metric-${label}`}
      className="rounded border border-slate-100 bg-slate-50 px-2 py-1.5"
    >
      <div className="text-[10px] text-slate-400">{label}</div>
      <div className="text-sm font-semibold text-slate-700">{value}</div>
    </div>
  );
}
