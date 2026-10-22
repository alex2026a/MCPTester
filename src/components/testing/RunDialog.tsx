import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { runTestCases, runTestSuite } from "../../api/tests";
import { toast } from "../../stores/toast";
import type { RunOptions } from "../../types/testing";
import { Field, Modal } from "./SuiteDialog";

interface Props {
  mode: "suite" | "selected";
  suiteId: string;
  serverId: string;
  caseIds: string[];
  caseCount: number;
  onClose: () => void;
  onStarted: (runId: string) => void;
}

export function RunDialog({
  mode,
  suiteId,
  serverId,
  caseIds,
  caseCount,
  onClose,
  onStarted,
}: Props) {
  const [stopOnFailure, setStopOnFailure] = useState(false);
  const [retryCount, setRetryCount] = useState(0);
  const [timeoutSecs, setTimeoutSecs] = useState("");
  const [allowWrite, setAllowWrite] = useState(false);
  const [allowDestructive, setAllowDestructive] = useState(false);
  const [allowUnknown, setAllowUnknown] = useState(false);
  const [postCaseHealth, setPostCaseHealth] = useState(false);

  const start = useMutation({
    mutationFn: async () => {
      const options: RunOptions = {
        case_ids: mode === "selected" ? caseIds : null,
        stop_on_failure: stopOnFailure,
        retry_count: retryCount,
        timeout_secs: timeoutSecs ? Number(timeoutSecs) : null,
        allow_write: allowWrite,
        allow_destructive: allowDestructive,
        allow_unknown: allowUnknown,
        post_case_health: postCaseHealth,
        seed: 20261021,
      };
      if (mode === "selected") {
        return runTestCases(serverId, caseIds, options);
      }
      return runTestSuite(suiteId, options);
    },
    onSuccess: (runId) => {
      toast.info("测试已开始");
      onStarted(runId);
      onClose();
    },
    onError: (error) => toast.fromError(error, "启动测试失败"),
  });

  return (
    <Modal
      title={mode === "suite" ? "运行套件" : `运行选中用例（${caseCount} 条）`}
      onClose={onClose}
    >
      <div className="space-y-3 text-sm">
        <p className="rounded border border-amber-200 bg-amber-50 px-3 py-2 text-[11px] text-amber-700">
          默认串行执行；write/unknown 需授权，destructive 默认跳过。生产标签服务器禁止
          destructive。
        </p>
        <div className="grid grid-cols-2 gap-3 text-xs">
          <Field label="单用例超时（秒，可空）">
            <input
              className="w-full rounded border border-slate-200 px-2 py-1.5"
              value={timeoutSecs}
              onChange={(event) => setTimeoutSecs(event.target.value)}
              inputMode="numeric"
              placeholder="使用设置页全局超时"
            />
          </Field>
          <Field label="环境错误重试次数">
            <input
              type="number"
              min={0}
              max={3}
              className="w-full rounded border border-slate-200 px-2 py-1.5"
              value={retryCount}
              onChange={(event) => setRetryCount(Number(event.target.value))}
            />
          </Field>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={stopOnFailure}
              onChange={(event) => setStopOnFailure(event.target.checked)}
            />
            失败即停
          </label>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={postCaseHealth}
              onChange={(event) => setPostCaseHealth(event.target.checked)}
            />
            每 20 条健康探测
          </label>
          <label className="flex items-center gap-2 text-amber-700">
            <input
              type="checkbox"
              checked={allowWrite}
              onChange={(event) => setAllowWrite(event.target.checked)}
            />
            允许写操作（write）
          </label>
          <label className="flex items-center gap-2 text-amber-700">
            <input
              type="checkbox"
              checked={allowUnknown}
              onChange={(event) => setAllowUnknown(event.target.checked)}
            />
            允许未知风险（unknown）
          </label>
          <label className="flex items-center gap-2 text-red-600">
            <input
              type="checkbox"
              checked={allowDestructive}
              onChange={(event) =>
                setAllowDestructive(event.target.checked)
              }
            />
            允许破坏性操作（destructive）
          </label>
        </div>
        <div className="flex justify-end gap-2 pt-2">
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
            disabled={start.isPending}
            onClick={() => start.mutate()}
          >
            {start.isPending ? "启动中…" : "开始运行"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
