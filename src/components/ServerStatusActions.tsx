import { useMutation, useQueryClient } from "@tanstack/react-query";
import { connectServer, reconnectServer } from "../api/servers";
import { toast } from "../stores/toast";
import type { ServerSummary } from "../types/models";
import { StatusBadge } from "./StatusBadge";

interface ServerStatusActionsProps {
  server?: ServerSummary;
  className?: string;
}

/** 当前服务器状态 + 就地连接/重连 */
export function ServerStatusActions({
  server,
  className,
}: ServerStatusActionsProps) {
  const queryClient = useQueryClient();

  const connect = useMutation({
    mutationFn: () => connectServer(server?.id ?? ""),
    onError: (error) => toast.fromError(error, "连接失败"),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["servers"] });
    },
  });

  const reconnect = useMutation({
    mutationFn: () => reconnectServer(server?.id ?? ""),
    onError: (error) => toast.fromError(error, "重连失败"),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["servers"] });
    },
  });

  if (!server) return null;

  const state = server.state.state;
  const oauthRequired =
    server.state.state === "error" &&
    server.state.error.kind === "oauth_required";
  const pending = connect.isPending || reconnect.isPending;
  const action =
    state === "disconnected"
      ? ("connect" as const)
      : state === "degraded" || state === "lost" || state === "error"
        ? ("reconnect" as const)
        : null;

  return (
    <span className={`inline-flex items-center gap-2 ${className ?? ""}`}>
      <StatusBadge state={server.state} />
      {oauthRequired ? (
        <span className="text-[11px] text-amber-600">
          需要登录；请到「服务器」页处理
        </span>
      ) : null}
      {state === "connecting" ? (
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs text-slate-400"
          disabled
        >
          连接中…
        </button>
      ) : action ? (
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
          disabled={pending}
          title={
            action === "connect"
              ? "连接服务器"
              : state === "degraded"
                ? "连接已降级，重连以恢复"
                : state === "lost"
                  ? "连接已掉线，重连以恢复"
                  : "连接出错，重连以恢复"
          }
          onClick={() => {
            if (action === "connect") connect.mutate();
            else reconnect.mutate();
          }}
        >
          {pending ? "处理中…" : action === "connect" ? "连接" : "重连"}
        </button>
      ) : null}
    </span>
  );
}
