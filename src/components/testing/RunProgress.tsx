import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { cancelTestRun } from "../../api/tests";
import { toast } from "../../stores/toast";

interface Progress {
  run_id: string;
  completed: number;
  total: number;
  passed: number;
  failed: number;
  warning: number;
  skipped: number;
  error: number;
  current?: string | null;
}

interface Finished {
  run_id: string;
  status: string;
  summary: {
    total: number;
    passed: number;
    failed: number;
    warning: number;
    skipped: number;
    error: number;
    canceled: number;
    pass_rate: number;
    effective_pass_rate: number;
  };
}

/** 运行进度：监听 test://run-* 事件，可取消。 */
export function RunProgress({
  runId,
  onClose,
  onFinished,
}: {
  runId: string;
  onClose: () => void;
  onFinished: (runId: string) => void;
}) {
  const [progress, setProgress] = useState<Progress | null>(null);
  const [finished, setFinished] = useState<Finished | null>(null);
  const [canceling, setCanceling] = useState(false);

  useEffect(() => {
    let disposed = false;
    const unlistenProgress = listen<Progress>("test://run-progress", (event) => {
      if (disposed || event.payload.run_id !== runId) return;
      setProgress(event.payload);
    });
    const unlistenFinished = listen<Finished>("test://run-finished", (event) => {
      if (disposed || event.payload.run_id !== runId) return;
      setFinished(event.payload);
      setProgress((current) => ({
        run_id: event.payload.run_id,
        completed: event.payload.summary.total,
        total: event.payload.summary.total,
        passed: event.payload.summary.passed,
        failed: event.payload.summary.failed,
        warning: event.payload.summary.warning,
        skipped: event.payload.summary.skipped,
        error: event.payload.summary.error,
        current: current?.current ?? null,
      }));
      toast.success("测试运行完成", {
        message: `通过率 ${(event.payload.summary.effective_pass_rate * 100).toFixed(1)}%`,
        action: {
          label: "查看报告",
          run: () => onFinished(runId),
        },
      });
    });
    const unlistenStarted = listen<{ run_id: string; total: number }>(
      "test://run-started",
      (event) => {
        if (disposed || event.payload.run_id !== runId) return;
        setProgress((current) => ({
          run_id: runId,
          completed: current?.completed ?? 0,
          total: event.payload.total,
          passed: current?.passed ?? 0,
          failed: current?.failed ?? 0,
          warning: current?.warning ?? 0,
          skipped: current?.skipped ?? 0,
          error: current?.error ?? 0,
          current: current?.current ?? null,
        }));
      },
    );
    return () => {
      disposed = true;
      void unlistenProgress.then((unlisten) => unlisten());
      void unlistenFinished.then((unlisten) => unlisten());
      void unlistenStarted.then((unlisten) => unlisten());
    };
  }, [runId, onFinished]);

  const total = progress?.total ?? 0;
  const completed = progress?.completed ?? 0;
  const percent = total > 0 ? Math.round((completed / total) * 100) : 0;

  return (
    <div className="rounded-lg border border-slate-200 bg-white p-3 text-xs">
      <div className="flex items-center gap-2">
        <span className="font-medium text-slate-700">
          {finished ? "运行完成" : "测试运行中"}
        </span>
        <span className="text-slate-400">
          {completed}/{total || "?"}
        </span>
        <span className="text-emerald-600">通过 {progress?.passed ?? 0}</span>
        <span className="text-red-600">失败 {progress?.failed ?? 0}</span>
        <span className="text-amber-600">警告 {progress?.warning ?? 0}</span>
        <span className="text-slate-500">
          跳过 {progress?.skipped ?? 0} · 错误 {progress?.error ?? 0}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {!finished ? (
            <button
              type="button"
              className="rounded border border-red-100 px-2 py-1 text-red-500 hover:bg-red-50 disabled:opacity-50"
              disabled={canceling}
              onClick={() => {
                setCanceling(true);
                void cancelTestRun(runId)
                  .then(() => toast.info("已请求取消"))
                  .catch((error) => toast.fromError(error, "取消失败"))
                  .finally(() => setCanceling(false));
              }}
            >
              取消
            </button>
          ) : (
            <button
              type="button"
              className="rounded bg-slate-800 px-2 py-1 text-white hover:bg-slate-700"
              onClick={() => onFinished(runId)}
            >
              查看报告
            </button>
          )}
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
            onClick={onClose}
          >
            关闭
          </button>
        </div>
      </div>
      <div className="mt-2 h-2 overflow-hidden rounded bg-slate-100">
        <div
          className={`h-full transition-all ${
            finished
              ? (progress?.failed ?? 0) + (progress?.error ?? 0) > 0
                ? "bg-red-500"
                : "bg-emerald-500"
              : "bg-sky-500"
          }`}
          style={{ width: `${percent}%` }}
        />
      </div>
      {progress?.current ? (
        <div className="mt-1 truncate text-slate-400">
          当前：{progress.current}
        </div>
      ) : null}
    </div>
  );
}
