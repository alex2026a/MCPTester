import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useSearchParams } from "react-router-dom";
import { EmptyState } from "../components/EmptyState";
import { ServerSelect } from "../components/ServerSelect";
import { LiveDashboard } from "../components/loadtest/LiveDashboard";
import { ProfileDialog } from "../components/loadtest/ProfileDialog";
import { RunResult } from "../components/loadtest/RunResult";
import { getActiveLoadTest, getLoadRun, listLoadRuns } from "../api/loadtest";
import { listServers } from "../api/servers";
import { listTestCases, listTestSuites } from "../api/tests";
import { pickDefaultServerId } from "../lib/serverOrder";
import type { LoadRunStatus } from "../types/loadtest";

const STATUS_LABEL: Record<LoadRunStatus, string> = {
  running: "运行中",
  completed: "完成",
  failed: "失败",
  canceled: "取消",
  interrupted: "中断",
  aborted: "自动中止",
};

const STATUS_CLASS: Record<LoadRunStatus, string> = {
  running: "bg-sky-50 text-sky-700",
  completed: "bg-emerald-50 text-emerald-600",
  failed: "bg-red-50 text-red-600",
  canceled: "bg-slate-100 text-slate-600",
  interrupted: "bg-amber-50 text-amber-700",
  aborted: "bg-red-50 text-red-600",
};

function formatTime(value: string): string {
  return new Date(value).toLocaleString();
}

/** 压测页：配置、实时看板、历史与结果。 */
export function LoadTest() {
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const [serverId, setServerId] = useState("");
  const [suiteId, setSuiteId] = useState("");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [hiddenRunId, setHiddenRunId] = useState<string | null>(null);
  const [localActive, setLocalActive] = useState<{
    runId: string;
    suiteName: string;
    profileName: string;
    startedAt: string;
  } | null>(null);

  const serverList = servers.data ?? [];
  const activeServerId = serverId || pickDefaultServerId(serverList);
  const activeServer = serverList.find((item) => item.id === activeServerId);
  const ready = activeServer?.state.state === "ready";

  const suites = useQuery({
    queryKey: ["test-suites", activeServerId],
    queryFn: () => listTestSuites(activeServerId || null),
    enabled: Boolean(activeServerId),
  });
  const suiteList = suites.data ?? [];
  const activeSuite =
    suiteList.find((item) => item.id === suiteId) ?? suiteList[0];

  const cases = useQuery({
    queryKey: ["test-cases", activeSuite?.id],
    queryFn: () => listTestCases(activeSuite?.id ?? ""),
    enabled: Boolean(activeSuite?.id),
  });
  const caseList = useMemo(
    () => (cases.data ?? []).filter((item) => item.enabled),
    [cases.data],
  );

  const active = useQuery({
    queryKey: ["active-load-run"],
    queryFn: getActiveLoadTest,
    refetchInterval: 5000,
  });
  const runs = useQuery({
    queryKey: ["load-runs", activeServerId],
    queryFn: () => listLoadRuns(activeServerId || null, 0, 50),
    refetchInterval: 5000,
  });
  const runList = runs.data?.items ?? [];
  const selectedRunId = searchParams.get("run") ?? runList[0]?.id ?? "";
  const detail = useQuery({
    queryKey: ["load-run", selectedRunId],
    queryFn: () => getLoadRun(selectedRunId),
    enabled: Boolean(selectedRunId),
  });

  const liveCandidate = localActive
    ? {
        runId: localActive.runId,
        suiteName: localActive.suiteName,
        profileName: localActive.profileName,
        startedAt: localActive.startedAt,
      }
    : active.data
      ? {
          runId: active.data.run_id,
          suiteName: active.data.suite_name,
          profileName: active.data.profile_name,
          startedAt: active.data.started_at,
        }
      : null;
  const live =
    liveCandidate && hiddenRunId !== liveCandidate.runId ? liveCandidate : null;
  const latest =
    live && active.data?.run_id === live.runId ? active.data.latest : null;

  function selectRun(runId: string) {
    setSearchParams({ run: runId }, { replace: true });
  }

  function refresh(runId?: string) {
    void queryClient.invalidateQueries({ queryKey: ["load-runs"] });
    void queryClient.invalidateQueries({ queryKey: ["active-load-run"] });
    // 运行结束后 detail 的 key 仍是同一个 run_id；必须显式失效，
    // 否则会继续显示启动时缓存的「运行中 + 0」。
    void queryClient.invalidateQueries({
      queryKey: runId ? ["load-run", runId] : ["load-run"],
    });
  }

  return (
    <section className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-xl font-semibold text-slate-800">压测</h2>
        <ServerSelect
          servers={serverList}
          value={activeServerId}
          onChange={(id) => {
            setServerId(id);
            setSuiteId("");
            setSearchParams({}, { replace: true });
          }}
          placeholder="选择服务器"
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          ariaLabel="选择压测服务器"
        />
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={activeSuite?.id ?? ""}
          onChange={(event) => {
            setSuiteId(event.target.value);
            setSearchParams({}, { replace: true });
          }}
          disabled={suiteList.length === 0}
          aria-label="选择压测套件"
        >
          {suiteList.length === 0 ? (
            <option value="">无套件</option>
          ) : (
            suiteList.map((suite) => (
              <option key={suite.id} value={suite.id}>
                {suite.name}（{suite.enabled_count}）
              </option>
            ))
          )}
        </select>
        <button
          type="button"
          className="rounded bg-slate-800 px-3 py-1 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
          disabled={!ready || !activeSuite || caseList.length === 0 || Boolean(live)}
          title={
            !ready
              ? "服务器未连接，请先连接"
              : !activeSuite
                ? "请先在测试页创建套件"
                : caseList.length === 0
                  ? "套件没有启用的用例"
                  : undefined
          }
          onClick={() => setDialogOpen(true)}
        >
          新建压测
        </button>
        <span className="ml-auto text-[11px] text-slate-400">
          本版为闭环固定/阶梯 VU；结果用于开发期摸底，不作为生产容量验收。
        </span>
      </div>

      {live ? (
        <LiveDashboard
          runId={live.runId}
          suiteName={live.suiteName}
          profileName={live.profileName}
          startedAt={live.startedAt}
          latest={latest}
          onFinished={(runId) => {
            setLocalActive(null);
            setHiddenRunId(runId);
            selectRun(runId);
            refresh(runId);
          }}
          onClose={() => {
            setHiddenRunId(live.runId);
            setLocalActive(null);
          }}
        />
      ) : null}

      <div className="grid min-h-0 flex-1 gap-3 lg:grid-cols-[280px_minmax(0,1fr)]">
        <div className="flex min-h-0 flex-col rounded-lg border border-slate-200 bg-white">
          <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2 text-xs font-medium text-slate-500">
            <span>压测历史</span>
            <button
              type="button"
              className="text-[11px] text-slate-400 hover:text-slate-600"
              onClick={() => refresh()}
            >
              刷新
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {runs.isLoading ? (
              <div className="p-2 text-xs text-slate-400">加载中…</div>
            ) : runList.length === 0 ? (
              <div className="p-2 text-xs text-slate-400">还没有压测运行</div>
            ) : (
              <div className="space-y-1">
                {runList.map((run) => (
                  <button
                    key={run.id}
                    type="button"
                    className={`w-full rounded border px-2 py-1.5 text-left text-xs ${
                      selectedRunId === run.id
                        ? "border-slate-300 bg-slate-50"
                        : "border-transparent hover:bg-slate-50"
                    }`}
                    onClick={() => selectRun(run.id)}
                  >
                    <div className="flex items-center gap-1">
                      <span className="min-w-0 flex-1 truncate font-medium text-slate-700">
                        {run.profile_name}
                      </span>
                      <span
                        className={`rounded px-1.5 py-0.5 text-[10px] ${STATUS_CLASS[run.status]}`}
                      >
                        {STATUS_LABEL[run.status]}
                      </span>
                    </div>
                    <div className="mt-0.5 truncate text-[10px] text-slate-400">
                      {run.suite_name} · {formatTime(run.started_at)}
                    </div>
                    <div className="mt-0.5 flex gap-2 text-[10px] text-slate-500">
                      <span>{run.total} 次</span>
                      <span>{(run.success_rate * 100).toFixed(0)}%</span>
                      <span>{run.rps.toFixed(1)} req/s</span>
                      <span>p95 {run.p95_ms}ms</span>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="min-h-0 overflow-y-auto">
          {detail.isLoading ? (
            <div className="p-4 text-xs text-slate-400">加载结果…</div>
          ) : detail.data ? (
            <RunResult run={detail.data} />
          ) : (
            <EmptyState
              title="还没有压测结果"
              description="连接服务器、选择测试套件后，点击「新建压测」开始。"
            />
          )}
        </div>
      </div>

      {dialogOpen && activeServerId && activeSuite ? (
        <ProfileDialog
          serverId={activeServerId}
          suiteId={activeSuite.id}
          suiteName={activeSuite.name}
          cases={caseList}
          onClose={() => setDialogOpen(false)}
          onStarted={(runId, profileName) => {
            setHiddenRunId(null);
            setLocalActive({
              runId,
              suiteName: activeSuite.name,
              profileName,
              startedAt: new Date().toISOString(),
            });
            refresh();
          }}
        />
      ) : null}
    </section>
  );
}
