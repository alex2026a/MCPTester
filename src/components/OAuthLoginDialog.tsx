import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  cancelOAuthLogin,
  listenOAuth,
  startOAuthLogin,
  type OAuthLoginStart,
} from "../api/oauth";
import { toErrorInfo } from "../lib/errors";

interface OAuthLoginDialogProps {
  serverId: string;
  serverName: string;
  onClose: () => void;
  onAuthorized: () => void;
}

type Phase = "starting" | "waiting" | "error" | "canceled";

/** OAuth 登录等待弹层（v0.3.0） */
export function OAuthLoginDialog({
  serverId,
  serverName,
  onClose,
  onAuthorized,
}: OAuthLoginDialogProps) {
  const [start, setStart] = useState<OAuthLoginStart | null>(null);
  const [phase, setPhase] = useState<Phase>("starting");
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const flowIdRef = useRef<string | null>(null);
  const closedRef = useRef(false);
  const onAuthorizedRef = useRef(onAuthorized);
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onAuthorizedRef.current = onAuthorized;
    onCloseRef.current = onClose;
  }, [onAuthorized, onClose]);

  const begin = useCallback(async () => {
    setPhase("starting");
    setError(null);
    setCopied(false);
    try {
      const result = await startOAuthLogin(serverId);
      if (closedRef.current) {
        void cancelOAuthLogin(result.flow_id).catch(() => {});
        return;
      }
      flowIdRef.current = result.flow_id;
      setStart(result);
      setPhase("waiting");
      if (!result.browser_opened) {
        setError(
          result.open_error ??
            "未能自动打开系统浏览器，请复制授权链接手动打开。",
        );
      }
    } catch (err) {
      const info = toErrorInfo(err);
      setError(info.message);
      setPhase("error");
    }
  }, [serverId]);

  // 启动登录；卸载时取消仍在等待的 flow
  useEffect(() => {
    closedRef.current = false;
    // 放到定时器中启动：避免在 effect 体内同步 setState（react-hooks 规则）
    const timer = window.setTimeout(() => {
      void begin();
    }, 0);
    return () => {
      window.clearTimeout(timer);
      closedRef.current = true;
      const flowId = flowIdRef.current;
      flowIdRef.current = null;
      if (flowId) void cancelOAuthLogin(flowId).catch(() => {});
    };
  }, [begin]);

  // 只处理当前 server + flow 的事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listenOAuth((payload) => {
      if (payload.server_id !== serverId) return;
      if (flowIdRef.current && payload.flow_id !== flowIdRef.current) return;
      if (payload.state === "authorized") {
        flowIdRef.current = null;
        onAuthorizedRef.current();
        onCloseRef.current();
      } else if (payload.state === "error") {
        setError(payload.message ?? "OAuth 登录失败");
        setPhase("error");
      } else {
        setPhase("canceled");
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [serverId]);

  function close() {
    const flowId = flowIdRef.current;
    flowIdRef.current = null;
    if (flowId) void cancelOAuthLogin(flowId).catch(() => {});
    onClose();
  }

  async function copyUrl() {
    if (!start) return;
    try {
      await navigator.clipboard.writeText(start.auth_url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <div className="w-full max-w-2xl space-y-4 rounded-lg bg-white p-5 shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-slate-800">
              OAuth 登录 · {serverName}
            </h3>
            <p className="mt-1 text-xs text-slate-500">
              将在系统浏览器完成授权；本窗口只等待回调，不会接触密码。
            </p>
          </div>
          <button
            type="button"
            className="text-sm text-slate-400 hover:text-slate-600"
            onClick={close}
            aria-label="关闭登录窗口"
          >
            ×
          </button>
        </div>

        {phase === "starting" ? (
          <p className="text-sm text-slate-500">正在准备授权链接…</p>
        ) : null}

        {start ? (
          <div className="space-y-2">
            <label className="block space-y-1 text-xs text-slate-600">
              授权链接
              <textarea
                readOnly
                className="h-20 w-full resize-none rounded border border-slate-200 bg-slate-50 p-2 font-mono text-[11px] text-slate-600"
                value={start.auth_url}
                onFocus={(event) => event.currentTarget.select()}
              />
            </label>
            <p className="text-[11px] text-slate-500">
              回调地址：{start.redirect_uri} · 超时{" "}
              {Math.round(start.flow_timeout_secs / 60)} 分钟
            </p>
            <p className="text-[11px] text-slate-500">
              请求权限：
              {(start.scopes ?? []).length > 0
                ? (start.scopes ?? []).join(" ")
                : "由授权服务器决定"}
            </p>
            {start.callback_warning ? (
              <p className="rounded border border-amber-200 bg-amber-50 px-2 py-1 text-[11px] text-amber-700">
                {start.callback_warning}
              </p>
            ) : null}
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                className="rounded border border-slate-300 px-3 py-1.5 text-xs text-slate-700 hover:bg-slate-50"
                onClick={() => void openUrl(start.auth_url).catch(() => {})}
              >
                重新打开浏览器
              </button>
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-50"
                onClick={() => void copyUrl()}
              >
                {copied ? "已复制" : "复制链接"}
              </button>
            </div>
          </div>
        ) : null}

        {phase === "waiting" ? (
          <p className="text-xs text-amber-600">
            正在等待浏览器授权完成…如浏览器未自动打开，请复制上方链接。
          </p>
        ) : null}

        {error ? (
          <div className="rounded border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-600">
            {error}
          </div>
        ) : null}

        {phase === "canceled" ? (
          <p className="text-xs text-slate-500">登录已取消。</p>
        ) : null}

        <div className="flex items-center justify-end gap-2">
          {phase === "error" || phase === "canceled" ? (
            <button
              type="button"
              className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700"
              onClick={() => void begin()}
            >
              重试
            </button>
          ) : null}
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 text-sm text-slate-600 hover:bg-slate-50"
            onClick={close}
          >
            {phase === "waiting" ? "取消登录" : "关闭"}
          </button>
        </div>
      </div>
    </div>
  );
}
