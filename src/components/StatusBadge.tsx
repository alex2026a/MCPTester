import { errorTitle, toErrorInfo } from "../lib/errors";
import type { ConnectionState } from "../types/models";

const LABELS: Record<string, { text: string; color: string }> = {
  disconnected: { text: "未连接", color: "bg-conn-idle" },
  connecting: { text: "连接中", color: "bg-conn-connecting animate-pulse" },
  ready: { text: "已连接", color: "bg-conn-ok" },
  degraded: { text: "降级", color: "bg-conn-degraded" },
  lost: { text: "掉线", color: "bg-conn-lost" },
  error: { text: "错误", color: "bg-conn-lost" },
};

/** 连接状态徽标：色点 + 文本（错误态显示人话标题，v0.1.1 不再暴露内部枚举名） */
export function StatusBadge({ state }: { state: ConnectionState }) {
  const label = LABELS[state.state] ?? LABELS.disconnected;
  const detail =
    state.state === "ready"
      ? `${state.handshake.protocol_version} · ${state.handshake.lifecycle === "modern" ? "现代世代" : "旧世代"}`
      : state.state === "error"
        ? errorTitle(state.error.kind)
        : state.state === "lost" || state.state === "degraded"
          ? state.reason
          : "";
  const tooltip =
    state.state === "error" ? toErrorInfo(state.error).message : undefined;
  return (
    <span
      className="inline-flex items-center gap-1.5 text-xs text-slate-500"
      title={tooltip}
    >
      <span className={`h-2.5 w-2.5 rounded-full ${label.color}`} />
      {label.text}
      {detail ? <span className="text-slate-400">· {detail}</span> : null}
    </span>
  );
}
