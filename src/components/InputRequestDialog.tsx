/**
 * 交互输入对话框（v0.6.0）
 *
 * - 表单模式复用 SchemaForm；URL 模式只打开链接并确认；
 * - 多条请求一次提交；每条可接受或拒绝；
 * - 最小化只隐藏，不卸载，保留表单状态。
 */

import { useId, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SchemaForm, type SchemaFormHandle } from "./SchemaForm";
import { toast } from "../stores/toast";
import type { InputAction, InputAnswer } from "../types/input";
import type { PendingInput } from "../stores/inputRequests";

function expiresText(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString("zh-CN", { hour12: false });
}

export function InputRequestDialog({
  item,
  visible,
  onRespond,
  onCancel,
  onMinimize,
}: {
  item: PendingInput;
  visible: boolean;
  onRespond: (answers: InputAnswer[]) => Promise<void>;
  onCancel: () => Promise<void>;
  onMinimize: () => void;
}) {
  const titleId = useId();
  const [actions, setActions] = useState<Record<string, InputAction>>(() =>
    Object.fromEntries(item.requests.map((request) => [request.key, "accept"])),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const forms = useRef<Record<string, SchemaFormHandle | null>>({});

  const target = item.context.target
    ? `${item.context.server_name} · ${item.context.target}`
    : item.context.server_name;
  const expires = expiresText(item.expiresAt);
  const round = item.context.round > 1 ? `第 ${item.context.round} 轮 · ` : "";

  function setAction(key: string, action: InputAction) {
    setActions((current) => ({ ...current, [key]: action }));
  }

  function openLink(url: string) {
    try {
      const parsed = new URL(url);
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        toast.error("不支持的链接协议", {
          message: "只允许打开 http / https 链接。",
        });
        return;
      }
    } catch {
      toast.error("链接格式无效", { message: url });
      return;
    }
    void openUrl(url).catch((reason) => {
      toast.fromError(reason, "打开链接失败");
    });
  }

  async function submit() {
    setError(null);
    const answers: InputAnswer[] = [];
    for (const request of item.requests) {
      const action = actions[request.key] ?? "accept";
      if (action === "decline") {
        answers.push({ key: request.key, action });
        continue;
      }
      if (request.mode === "url") {
        answers.push({ key: request.key, action });
        continue;
      }
      const form = forms.current[request.key];
      if (!form) {
        setError("表单尚未准备好，请重试");
        return;
      }
      const result = form.validate();
      if (result.parseError) {
        setError(result.parseError);
        return;
      }
      if (result.errors.length > 0) {
        setError("请先修正表单错误");
        return;
      }
      answers.push({ key: request.key, action, content: result.value });
    }

    setBusy(true);
    try {
      await onRespond(answers);
    } catch (reason) {
      toast.fromError(reason, "提交输入失败");
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    setBusy(true);
    try {
      await onCancel();
    } catch (reason) {
      toast.fromError(reason, "取消调用失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className={
        visible
          ? "fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4"
          : "hidden"
      }
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !busy) {
          event.stopPropagation();
          onMinimize();
        }
      }}
    >
      <div className="flex max-h-[calc(100vh-2rem)] w-full max-w-2xl flex-col overflow-hidden rounded-xl bg-white shadow-2xl">
        <header className="border-b border-slate-200 px-4 py-3">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 id={titleId} className="text-base font-semibold text-slate-800">
                服务器请求输入
              </h2>
              <p className="mt-0.5 text-xs text-slate-500">
                {round}
                {target}
                {expires ? ` · 有效期至 ${expires}` : ""}
              </p>
            </div>
            <button
              type="button"
              className="rounded border border-slate-200 px-2 py-1 text-xs text-slate-600 hover:bg-slate-50"
              onClick={onMinimize}
              disabled={busy}
            >
              稍后处理
            </button>
          </div>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
          <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
            输入内容会写入本地历史；表单模式的敏感字段会掩码，JSON 模式不掩码，请避免粘贴真实凭据。
          </div>
          <div className="space-y-3">
            {item.requests.map((request) => {
              const action = actions[request.key] ?? "accept";
              return (
                <section
                  key={request.key}
                  className="rounded-lg border border-slate-200 p-3"
                >
                  <div className="mb-2 flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-[11px] text-slate-400">
                        {request.mode === "url" ? "URL 输入" : "表单输入"}
                      </div>
                      <p className="whitespace-pre-wrap text-sm text-slate-700">
                        {request.message}
                      </p>
                    </div>
                    <div className="flex shrink-0 gap-1 text-xs">
                      <button
                        type="button"
                        aria-pressed={action === "accept"}
                        className={`rounded px-2 py-1 ${
                          action === "accept"
                            ? "bg-slate-800 text-white"
                            : "border border-slate-200 text-slate-600 hover:bg-slate-50"
                        }`}
                        onClick={() => setAction(request.key, "accept")}
                        disabled={busy}
                      >
                        接受
                      </button>
                      <button
                        type="button"
                        aria-pressed={action === "decline"}
                        className={`rounded px-2 py-1 ${
                          action === "decline"
                            ? "bg-amber-600 text-white"
                            : "border border-slate-200 text-slate-600 hover:bg-slate-50"
                        }`}
                        onClick={() => setAction(request.key, "decline")}
                        disabled={busy}
                      >
                        拒绝
                      </button>
                    </div>
                  </div>

                  {action === "decline" ? (
                    <p className="text-xs text-slate-500">
                      将向服务器返回拒绝；服务器可能降级或继续执行。
                    </p>
                  ) : request.mode === "url" ? (
                    <div className="space-y-2">
                      <button
                        type="button"
                        className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
                        onClick={() => request.url && openLink(request.url)}
                        disabled={!request.url || busy}
                      >
                        打开链接
                      </button>
                      <div className="break-all text-xs text-slate-500">
                        {request.url ?? "服务器未提供 URL"}
                      </div>
                      <p className="text-xs text-slate-500">
                        完成后回到这里点「提交」确认。
                      </p>
                    </div>
                  ) : (
                    <SchemaForm
                      ref={(handle) => {
                        forms.current[request.key] = handle;
                      }}
                      schema={request.schema ?? {}}
                      onChange={() => {}}
                      maskSensitive
                    />
                  )}
                </section>
              );
            })}
          </div>
          {error ? (
            <div role="alert" className="mt-3 text-xs text-red-600">
              {error}
            </div>
          ) : null}
        </div>

        <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-slate-200 px-4 py-3">
          <button
            type="button"
            className="rounded border border-red-200 px-3 py-1.5 text-xs text-red-600 hover:bg-red-50 disabled:opacity-50"
            onClick={() => void cancel()}
            disabled={busy}
          >
            取消调用
          </button>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="rounded border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-50 disabled:opacity-50"
              onClick={onMinimize}
              disabled={busy}
            >
              稍后处理
            </button>
            <button
              type="button"
              className="rounded bg-slate-800 px-4 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
              onClick={() => void submit()}
              disabled={busy}
            >
              {busy
                ? "提交中…"
                : item.requests.length > 1
                  ? `提交全部（${item.requests.length}）`
                  : "提交"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
