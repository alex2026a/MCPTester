import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { ErrorDetailDialog } from "../components/ErrorDetailDialog";
import { ExportPreviewDialog } from "../components/ExportPreviewDialog";
import { ImportDialog } from "../components/ImportDialog";
import { ServerConfigDialog } from "../components/ServerConfigDialog";
import { ServerForm } from "../components/ServerForm";
import { StatusBadge } from "../components/StatusBadge";
import { getAppInfo } from "../api/app";
import {
  addServer,
  connectServer,
  deleteServer,
  disconnectServer,
  exportServers,
  getServer,
  listServers,
  previewExportServers,
  probeServer,
  reconnectServer,
  testServerConnection,
  updateServer,
  type ImportReport,
} from "../api/servers";
import { toErrorInfo, type ErrorInfo } from "../lib/errors";
import { toast } from "../stores/toast";
import type { ServerConfig, ServerDraft, TransportKind } from "../types/models";

const TRANSPORT_LABEL: Record<TransportKind, string> = {
  stdio: "stdio",
  streamable_http: "HTTP",
  sse: "SSE（旧协议）",
};

/** 服务器页：列表 + 连接管理 + 新建/编辑 */
export function Servers() {
  const queryClient = useQueryClient();
  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const appInfo = useQuery({ queryKey: ["appInfo"], queryFn: getAppInfo });

  const [showForm, setShowForm] = useState(false);
  const [editing, setEditing] = useState<ServerConfig | null>(null);
  const [detail, setDetail] = useState<ErrorInfo | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportFormat, setExportFormat] = useState<"claude" | "vscode">(
    "claude",
  );
  const [configJson, setConfigJson] = useState<{
    id: string;
    name: string;
  } | null>(null);

  const exportLoad = useCallback(
    () => previewExportServers(exportFormat),
    [exportFormat],
  );

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["servers"] });
  };

  const save = useMutation({
    mutationFn: async (draft: ServerDraft) => {
      if (editing) return updateServer(editing.id, draft);
      return addServer(draft);
    },
    onSuccess: () => {
      setShowForm(false);
      setEditing(null);
      invalidate();
    },
    onError: (error) => toast.fromError(error, "保存失败"),
  });

  const connect = useMutation({
    mutationFn: (id: string) => connectServer(id),
    onSuccess: invalidate,
    onError: (error) => {
      toast.fromError(error);
      invalidate();
    },
  });
  const disconnect = useMutation({
    mutationFn: (id: string) => disconnectServer(id),
    onSuccess: invalidate,
  });
  const reconnect = useMutation({
    mutationFn: (id: string) => reconnectServer(id),
    onSuccess: invalidate,
    onError: (error) => {
      toast.fromError(error);
      invalidate();
    },
  });
  const probe = useMutation({
    mutationFn: (id: string) => probeServer(id),
    onSuccess: (result) => {
      if (result.ok) {
        toast.success(`探测成功（${result.rtt_ms}ms）`, {
          message: result.detail,
        });
      } else {
        toast.error("探测失败", { message: result.detail, sticky: true });
      }
    },
    onError: (error) => toast.fromError(error, "探测失败"),
  });
  const remove = useMutation({
    mutationFn: (id: string) => deleteServer(id),
    onSuccess: invalidate,
    onError: (error) => toast.fromError(error, "删除失败"),
  });

  async function startEdit(id: string) {
    const config = await getServer(id);
    setEditing(config);
    setShowForm(true);
  }

  return (
    <section className="flex h-full flex-col">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <h2 className="text-xl font-semibold text-slate-800">服务器</h2>
          {appInfo.data?.portable ? (
            <span
              className="rounded bg-emerald-50 px-1.5 py-0.5 text-[11px] text-emerald-700"
              title={`便携数据目录：${appInfo.data.data_dir}`}
            >
              便携模式
            </span>
          ) : null}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 text-sm hover:bg-slate-50"
            onClick={() => setImportOpen(true)}
          >
            导入
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-3 py-1.5 text-sm hover:bg-slate-50"
            onClick={() => setExportOpen(true)}
          >
            导出
          </button>
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700"
            onClick={() => {
              setEditing(null);
              setShowForm(true);
            }}
          >
            新建服务器
          </button>
        </div>
      </div>

      {appInfo.data?.config_error ? (
        <div className="mb-3 rounded border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-600">
          启动配置：{appInfo.data.config_error}
        </div>
      ) : appInfo.data?.config_changed ? (
        <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700">
          启动配置文件已变化；seed
          模式不会覆盖已有服务器。可在「设置」页重新应用配置。
        </div>
      ) : null}

      {showForm ? (
        <div className="mb-4">
          <ServerForm
            initial={editing}
            submitting={save.isPending}
            plaintextCredentials={Boolean(
              appInfo.data?.portable || editing?.origin,
            )}
            onSubmit={(draft) => save.mutate(draft)}
            onTest={(draft) => testServerConnection(draft, editing?.id ?? null)}
            onCancel={() => {
              setShowForm(false);
              setEditing(null);
            }}
          />
        </div>
      ) : null}

      <div className="flex-1 space-y-3 overflow-y-auto">
        {servers.isLoading ? (
          <div className="text-sm text-slate-400">加载中…</div>
        ) : servers.isError ? (
          <div className="text-sm text-red-500">
            加载失败：{String(servers.error)}
          </div>
        ) : (servers.data ?? []).length === 0 ? (
          <EmptyState
            title="还没有服务器"
            description="点击右上角「新建服务器」添加 stdio / HTTP / SSE 服务器，然后连接并开始测试接口。"
          />
        ) : (
          (servers.data ?? []).map((server) => {
            const ready = server.state.state === "ready";
            const busy =
              connect.isPending || disconnect.isPending || reconnect.isPending;
            const stateError =
              server.state.state === "error"
                ? toErrorInfo(server.state.error)
                : null;
            return (
              <div
                key={server.id}
                className="flex items-center justify-between rounded-lg border border-slate-200 bg-white p-3"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium text-slate-800">
                      {server.name}
                    </span>
                    <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[11px] text-slate-500">
                      {TRANSPORT_LABEL[server.transport]}
                    </span>
                    {server.origin?.source === "config" ? (
                      <span
                        className="rounded bg-sky-50 px-1.5 py-0.5 text-[11px] text-sky-600"
                        title="由 mcptester.config.json 预置；凭据以明文保存在本地数据库"
                      >
                        配置文件
                      </span>
                    ) : null}
                  </div>
                  <StatusBadge state={server.state} />
                  {stateError ? (
                    <button
                      type="button"
                      className="block max-w-xl truncate text-left text-[11px] text-red-500 hover:underline"
                      title={stateError.message}
                      onClick={() => setDetail(stateError)}
                    >
                      {stateError.message}（详情）
                    </button>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-2 text-xs">
                  {ready ? (
                    <>
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                        onClick={() => disconnect.mutate(server.id)}
                        disabled={busy}
                      >
                        断开
                      </button>
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                        onClick={() => probe.mutate(server.id)}
                      >
                        探测
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                      onClick={() => connect.mutate(server.id)}
                      disabled={busy}
                    >
                      {server.state.state === "connecting" ? "连接中…" : "连接"}
                    </button>
                  )}
                  {server.state.state === "lost" ||
                  server.state.state === "error" ? (
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                      onClick={() => reconnect.mutate(server.id)}
                      disabled={busy}
                    >
                      重连
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                    onClick={() =>
                      setConfigJson({ id: server.id, name: server.name })
                    }
                  >
                    JSON
                  </button>
                  <button
                    type="button"
                    className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                    onClick={() => void startEdit(server.id)}
                  >
                    编辑
                  </button>
                  <button
                    type="button"
                    className="rounded border border-red-100 px-2 py-1 text-red-500 hover:bg-red-50"
                    onClick={() => {
                      if (
                        window.confirm(
                          `确定删除服务器「${server.name}」？（历史记录保留）`,
                        )
                      ) {
                        remove.mutate(server.id);
                      }
                    }}
                  >
                    删除
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>

      {detail ? (
        <ErrorDetailDialog info={detail} onClose={() => setDetail(null)} />
      ) : null}

      {importOpen ? (
        <ImportDialog
          onClose={() => setImportOpen(false)}
          onImported={(report: ImportReport) => {
            toast.success(
              `导入完成：新增 ${report.added}，跳过 ${report.skipped.length}，失败 ${report.failed.length}`,
            );
            setImportOpen(false);
            invalidate();
          }}
        />
      ) : null}

      {exportOpen ? (
        <ExportPreviewDialog
          key={exportFormat}
          title="导出服务器配置"
          defaultFilename={`mcptester-servers-${new Date()
            .toISOString()
            .slice(0, 10)
            .replace(/-/g, "")}.json`}
          load={exportLoad}
          onSave={(path) => exportServers(exportFormat, path)}
          onClose={() => setExportOpen(false)}
          extra={
            <select
              className="rounded border border-slate-200 px-2 py-1 text-xs"
              value={exportFormat}
              onChange={(event) =>
                setExportFormat(event.target.value as "claude" | "vscode")
              }
            >
              <option value="claude">Claude / Cursor 格式（mcpServers）</option>
              <option value="vscode">VS Code 格式（servers）</option>
            </select>
          }
        />
      ) : null}

      {configJson ? (
        <ServerConfigDialog
          serverId={configJson.id}
          serverName={configJson.name}
          onClose={() => setConfigJson(null)}
        />
      ) : null}
    </section>
  );
}
