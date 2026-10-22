import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getAppInfo,
  openConfigDir,
  openDataDir,
  reloadBootstrapConfig,
} from "../api/app";
import { getSettings, updateSettings, type Settings } from "../api/settings";
import { zh } from "../i18n/zh";

/** 设置页：超时 / 保留策略 / 缓冲 / 自动重连 + 运行目录与启动配置 */
export function SettingsPage() {
  const queryClient = useQueryClient();
  const appInfo = useQuery({ queryKey: ["appInfo"], queryFn: getAppInfo });
  const settings = useQuery({ queryKey: ["settings"], queryFn: getSettings });
  const [draft, setDraft] = useState<Settings | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // 编辑草稿优先；未编辑时直接使用已加载设置（避免 effect 同步状态）
  const effective: Settings | null = draft ?? settings.data ?? null;

  const save = useMutation({
    mutationFn: (next: Settings) => updateSettings(next),
    onSuccess: (saved) => {
      queryClient.setQueryData(["settings"], saved);
      setNotice(
        "已保存（超时/缓冲类设置在下次连接时生效；保留策略在下次启动清理时生效）",
      );
    },
    onError: (error) => setNotice(String(error)),
  });

  const reloadConfig = useMutation({
    mutationFn: reloadBootstrapConfig,
    onSuccess: (report) => {
      void queryClient.invalidateQueries({ queryKey: ["appInfo"] });
      void queryClient.invalidateQueries({ queryKey: ["servers"] });
      const firstFailure = report.failed[0];
      if (report.status === "not_found") {
        setNotice("未找到启动配置文件");
      } else if (report.status === "error") {
        setNotice(`配置应用失败：${firstFailure?.reason ?? "未知错误"}`);
      } else {
        setNotice(
          `配置已重新应用：新增 ${report.added}，跳过 ${report.skipped}，失败 ${report.failed.length}${
            firstFailure
              ? `（首个失败：${firstFailure.key}：${firstFailure.reason}）`
              : ""
          }`,
        );
      }
    },
    onError: (error) => setNotice(String(error)),
  });

  return (
    <section className="flex h-full flex-col">
      <h2 className="mb-4 text-xl font-semibold text-slate-800">
        {zh.settings.title}
      </h2>

      <div className="max-w-2xl space-y-4">
        <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4 text-sm">
          <div className="text-xs font-medium text-slate-500">调用与连接</div>
          {effective ? (
            <div className="grid grid-cols-2 gap-3">
              <NumberField
                label="调用超时（秒）"
                value={effective.request_timeout_secs}
                onChange={(value) =>
                  setDraft({ ...effective, request_timeout_secs: value })
                }
              />
              <NumberField
                label="连接超时（秒）"
                value={effective.connect_timeout_secs}
                onChange={(value) =>
                  setDraft({ ...effective, connect_timeout_secs: value })
                }
              />
              <label className="flex items-center gap-2 text-xs text-slate-600">
                <input
                  type="checkbox"
                  checked={effective.auto_reconnect}
                  onChange={(e) =>
                    setDraft({ ...effective, auto_reconnect: e.target.checked })
                  }
                />
                掉线自动重连
              </label>
              <label className="flex items-center gap-2 text-xs text-slate-600">
                <input
                  type="checkbox"
                  checked={effective.auto_refresh_on_list_changed}
                  onChange={(e) =>
                    setDraft({
                      ...effective,
                      auto_refresh_on_list_changed: e.target.checked,
                    })
                  }
                />
                列表变更通知自动刷新
              </label>
            </div>
          ) : (
            <div className="text-xs text-slate-400">加载中…</div>
          )}
        </div>

        <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4 text-sm">
          <div className="text-xs font-medium text-slate-500">
            测试与报告保留
          </div>
          {effective ? (
            <div className="grid grid-cols-2 gap-3">
              <NumberField
                label="测试调用保留天数"
                value={effective.test_call_retention_days}
                onChange={(value) =>
                  setDraft({ ...effective, test_call_retention_days: value })
                }
              />
              <NumberField
                label="测试调用最大条数"
                value={effective.test_call_max_rows}
                onChange={(value) =>
                  setDraft({ ...effective, test_call_max_rows: value })
                }
              />
              <NumberField
                label="测试报告保留天数"
                value={effective.test_run_retention_days}
                onChange={(value) =>
                  setDraft({ ...effective, test_run_retention_days: value })
                }
              />
              <NumberField
                label="测试报告最大运行数"
                value={effective.test_run_max_rows}
                onChange={(value) =>
                  setDraft({ ...effective, test_run_max_rows: value })
                }
              />
            </div>
          ) : (
            <div className="text-xs text-slate-400">加载中…</div>
          )}
        </div>

        <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4 text-sm">
          <div className="text-xs font-medium text-slate-500">
            运行与启动配置
          </div>
          {appInfo.isLoading ? (
            <div className="text-xs text-slate-400">加载中…</div>
          ) : appInfo.isError || !appInfo.data ? (
            <div className="text-xs text-red-500">应用信息加载失败</div>
          ) : (
            <>
              <Row
                label="运行模式"
                value={
                  appInfo.data.portable
                    ? "便携模式（数据在程序目录 data/）"
                    : appInfo.data.data_dir_source === "env"
                      ? "自定义数据目录（MCPTESTER_DATA_DIR）"
                      : appInfo.data.data_dir_source === "fallback"
                        ? "安装模式（便携目录不可写，已回退）"
                        : "安装模式（数据在用户目录）"
                }
              />
              <Row
                label="数据目录"
                value={`${appInfo.data.data_dir}（${dataDirSourceLabel(appInfo.data.data_dir_source)}）`}
              />
              <Row label="日志目录" value={appInfo.data.log_dir} />
              <Row
                label="启动配置"
                value={
                  appInfo.data.config_path ?? "未找到 mcptester.config.json"
                }
              />
              <Row
                label="配置状态"
                value={configStatusLabel(appInfo.data.config_status)}
              />
              <Row
                label="默认服务器"
                value={appInfo.data.default_server_id ?? "-"}
              />
              <Row
                label="自动连接"
                value={
                  appInfo.data.auto_connect ? "启动时连接默认服务器" : "关闭"
                }
              />
              {appInfo.data.config_error ? (
                <div className="rounded border border-red-200 bg-red-50 px-2 py-1 text-xs text-red-600">
                  {appInfo.data.config_error}
                </div>
              ) : null}
              {appInfo.data.config_changed ? (
                <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1 text-xs text-amber-700">
                  配置文件在 seed 后发生变化。seed 模式不会自动覆盖已有服务器；
                  可点「重新应用配置」补回缺失项。
                </div>
              ) : null}
              {appInfo.data.data_dir_source === "fallback" ? (
                <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1 text-xs text-amber-700">
                  便携数据目录不可写，本次已回退到用户目录；设置项与凭据策略可能不同。
                </div>
              ) : null}
              {appInfo.data.portable ? (
                <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1 text-xs text-amber-700">
                  便携模式：服务器凭据以明文保存在{" "}
                  <code>data/mcptester.db</code>。请勿把含真实凭据的 data
                  目录对外分发。
                </div>
              ) : null}
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50 disabled:opacity-40"
                  disabled={reloadConfig.isPending}
                  onClick={() => reloadConfig.mutate()}
                >
                  {reloadConfig.isPending ? "应用中…" : "重新应用配置"}
                </button>
                <button
                  type="button"
                  className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
                  onClick={() => {
                    void openDataDir().catch(() => {});
                  }}
                >
                  打开数据目录
                </button>
                <button
                  type="button"
                  className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
                  onClick={() => {
                    void openConfigDir().catch(() => {});
                  }}
                >
                  打开配置位置
                </button>
              </div>
              <div className="text-[11px] text-slate-400">
                「重新应用配置」只补充缺失的预置服务器，不覆盖已有服务器。
              </div>
            </>
          )}
        </div>

        <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4 text-sm">
          <div className="text-xs font-medium text-slate-500">历史与缓冲</div>
          {effective ? (
            <div className="grid grid-cols-2 gap-3">
              <NumberField
                label="历史保留天数"
                value={effective.history_retention_days}
                onChange={(value) =>
                  setDraft({ ...effective, history_retention_days: value })
                }
              />
              <NumberField
                label="历史最大条数"
                value={effective.history_max_rows}
                onChange={(value) =>
                  setDraft({ ...effective, history_max_rows: value })
                }
              />
              <NumberField
                label="stderr 缓冲行数"
                value={effective.stderr_buffer_lines}
                onChange={(value) =>
                  setDraft({ ...effective, stderr_buffer_lines: value })
                }
              />
              <NumberField
                label="通知缓冲行数"
                value={effective.notification_buffer_lines}
                onChange={(value) =>
                  setDraft({ ...effective, notification_buffer_lines: value })
                }
              />
            </div>
          ) : null}
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={!effective || save.isPending}
            onClick={() => effective && save.mutate(effective)}
          >
            保存设置
          </button>
          {notice ? (
            <span className="text-xs text-amber-600">{notice}</span>
          ) : null}
        </div>

        <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4 text-sm">
          <div className="text-xs font-medium text-slate-500">关于</div>
          {appInfo.isLoading ? (
            <div className="text-slate-400">{zh.common.loading}</div>
          ) : appInfo.isError || !appInfo.data ? (
            <div className="text-red-500">{zh.common.loadFailed}</div>
          ) : (
            <>
              <Row
                label={zh.settings.version}
                value={`v${appInfo.data.version}`}
              />
              <Row
                label={zh.settings.protocolVersions}
                value={
                  appInfo.data.supported_protocol_versions.join(" / ") || "-"
                }
              />
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function NumberField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="space-y-1 text-xs text-slate-600">
      {label}
      <input
        type="number"
        min={0}
        className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </label>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start gap-3">
      <span className="w-24 shrink-0 text-slate-400">{label}</span>
      <span className="break-all text-slate-700">{value}</span>
    </div>
  );
}

function dataDirSourceLabel(source: string): string {
  switch (source) {
    case "env":
      return "环境变量";
    case "portable":
      return "便携目录";
    case "fallback":
      return "回退";
    default:
      return "用户目录";
  }
}

function configStatusLabel(status: string): string {
  switch (status) {
    case "not_found":
      return "未找到配置文件";
    case "pending":
      return "已发现，尚未应用";
    case "applied":
      return "已应用";
    case "partial":
      return "部分应用（有失败项）";
    case "error":
      return "解析/应用失败";
    default:
      return status;
  }
}
