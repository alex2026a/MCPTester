import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { cancelLoadTest } from "../../api/loadtest";
import { toast } from "../../stores/toast";
import type { LoadTick } from "../../types/loadtest";
import { SeriesChart } from "./SeriesChart";

interface Finished {
  run_id: string;
  status: string;
  stop_reason?: string | null;
}

interface Props {
  runId: string;
  suiteName: string;
  profileName: string;
  startedAt: string;
  latest?: LoadTick | null;
  onFinished: (runId: string) => void;
  onClose: () => void;
}

function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes > 0 ? `${minutes}m ${rest}s` : `${rest}s`;
}

/** 实时看板：监听 loadtest://tick，展示 RPS/分位/错误率。 */
export function LiveDashboard({
  runId,
  suiteName,
  profileName,
  startedAt,
  latest,
  onFinished,
  onClose,
}: Props) {
  const [ticks, setTicks] = useState<LoadTick[]>(latest ? [latest] : []);
  const [finished, setFinished] = useState<Finished | null>(null);
  const [stopping, setStopping] = useState(false);

  useEffect(() => {
    let disposed = false;
    const unlistenTick = listen<LoadTick>("loadtest://tick", (event) => {
      if (disposed || event.payload.run_id !== runId) return;
      setTicks((current) => [...current, event.payload].slice(-300));
    });
    const unlistenFinished = listen<Finished>(
      "loadtest://run-finished",
      (event) => {
        if (disposed || event.payload.run_id !== runId) return;
        setFinished(event.payload);
        toast.info("压测运行结束", {
          message: `状态：${event.payload.status}`,
        });
        onFinished(runId);
      },
    );
    return () => {
      disposed = true;
      void unlistenTick.then((unlisten) => unlisten());
      void unlistenFinished.then((unlisten) => unlisten());
    };
  }, [runId, onFinished]);

  const current = ticks[ticks.length - 1];
  const elapsed = current?.elapsed_ms ?? 0;
  const stageProgress =
    current && current.stage_count > 0
      ? `${Math.min(current.stage_index + 1, current.stage_count)}/${current.stage_count}`
      : "-";

  function stop() {
    if (!window.confirm("停止压测？已完成的统计会保留。")) return;
    setStopping(true);
    void cancelLoadTest(runId)
      .then(() => toast.info("已请求停止"))
      .catch((error) => toast.fromError(error, "停止压测失败"))
      .finally(() => setStopping(false));
  }

  return (
    <div className="rounded-lg border border-slate-200 bg-white p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-slate-700">压测运行中</span>
        <span className="text-xs text-slate-400">
          {suiteName} · {profileName} · {formatDuration(elapsed)} · 开始于{" "}
          {new Date(startedAt).toLocaleTimeString()}
        </span>
        <span className="rounded bg-sky-50 px-2 py-0.5 text-[11px] text-sky-700">
          阶段 {stageProgress}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {finished ? (
            <button
              type="button"
              className="rounded bg-slate-800 px-2 py-1 text-xs text-white hover:bg-slate-700"
              onClick={() => onFinished(runId)}
            >
              查看结果
            </button>
          ) : (
            <button
              type="button"
              className="rounded border border-red-100 px-2 py-1 text-xs text-red-500 hover:bg-red-50 disabled:opacity-50"
              disabled={stopping}
              onClick={stop}
            >
              {stopping ? "停止中…" : "停止"}
            </button>
          )}
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
            onClick={onClose}
          >
            收起
          </button>
        </div>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 md:grid-cols-6">
        <Metric label="RPS" value={current?.rps.toFixed(1) ?? "-"} />
        <Metric label="p95" value={current ? `${current.p95_ms}ms` : "-"} />
        <Metric label="p99" value={current ? `${current.p99_ms}ms` : "-"} />
        <Metric
          label="错误率"
          value={
            current ? `${(current.error_rate * 100).toFixed(1)}%` : "-"
          }
          tone={current && current.error_rate > 0.05 ? "bad" : undefined}
        />
        <Metric
          label="活动 VU"
          value={
            current ? `${current.active_vus}/${current.target_vus}` : "-"
          }
        />
        <Metric label="完成调用" value={current ? String(current.eligible) : "-"} />
      </div>

      <div className="mt-3 grid gap-2 md:grid-cols-2">
        <SeriesChart
          series={[{ label: "RPS", color: "#0284c7", values: ticks.map((tick) => tick.rps) }]}
          unit="req/s"
        />
        <SeriesChart
          series={[
            {
              label: "p95",
              color: "#7c3aed",
              values: ticks.map((tick) => tick.p95_ms),
            },
            {
              label: "p99",
              color: "#db2777",
              values: ticks.map((tick) => tick.p99_ms),
            },
          ]}
          unit="ms"
        />
        <SeriesChart
          series={[
            {
              label: "错误率",
              color: "#dc2626",
              values: ticks.map((tick) => tick.error_rate * 100),
            },
          ]}
          unit="%"
        />
        <div className="rounded border border-slate-200 bg-white p-2">
          <div className="text-[10px] text-slate-400">最近错误</div>
          <div className="mt-1 max-h-24 space-y-1 overflow-y-auto text-[11px]">
            {(current?.recent_errors ?? []).length === 0 ? (
              <div className="text-slate-400">暂无</div>
            ) : (
              current?.recent_errors.map((error, index) => (
                <div key={`${error.at}-${index}`} className="truncate text-red-600">
                  {error.case_name} · {error.status} · {error.message}
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "bad";
}) {
  return (
    <div className="rounded border border-slate-100 bg-slate-50 px-2 py-1.5">
      <div className="text-[10px] text-slate-400">{label}</div>
      <div
        className={`text-sm font-semibold ${
          tone === "bad" ? "text-red-600" : "text-slate-700"
        }`}
      >
        {value}
      </div>
    </div>
  );
}
