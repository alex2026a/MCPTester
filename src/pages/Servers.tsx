import {
  useCallback,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { ErrorDetailDialog } from "../components/ErrorDetailDialog";
import { ExportPreviewDialog } from "../components/ExportPreviewDialog";
import { ImportDialog } from "../components/ImportDialog";
import { OAuthDiagnosticsDialog } from "../components/OAuthDiagnosticsDialog";
import { OAuthLoginDialog } from "../components/OAuthLoginDialog";
import { ServerConfigDialog } from "../components/ServerConfigDialog";
import { ServerForm } from "../components/ServerForm";
import { StatusBadge } from "../components/StatusBadge";
import { getAppInfo } from "../api/app";
import { clearOAuthRegistration, logoutOAuth } from "../api/oauth";
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
  reorderServers,
  testServerConnection,
  updateServer,
  type ImportReport,
} from "../api/servers";
import { toErrorInfo, type ErrorInfo } from "../lib/errors";
import { moveItem } from "../lib/serverOrder";
import { toast } from "../stores/toast";
import type {
  ServerConfig,
  ServerDraft,
  ServerSummary,
  TransportKind,
} from "../types/models";

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
  const [oauthServer, setOauthServer] = useState<ServerSummary | null>(null);
  const [diagnosticsServer, setDiagnosticsServer] =
    useState<ServerSummary | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportFormat, setExportFormat] = useState<"claude" | "vscode">(
    "claude",
  );
  const [configJson, setConfigJson] = useState<{
    id: string;
    name: string;
  } | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const dragIdRef = useRef<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{
    id: string;
    after: boolean;
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
  const logout = useMutation({
    mutationFn: (id: string) => logoutOAuth(id),
    onSuccess: (result) => {
      toast.success(
        result.revoked
          ? "已退出并撤销授权"
          : "已退出登录（本地凭据已清除）",
        result.detail ? { message: result.detail } : undefined,
      );
      invalidate();
    },
    onError: (error) => toast.fromError(error, "退出登录失败"),
  });
  const clearRegistration = useMutation({
    mutationFn: (id: string) => clearOAuthRegistration(id),
    onSuccess: (result) => {
      toast.success(
        result.cleared
          ? "已清除客户端注册；下次登录会重新选择 DCR/CIMD"
          : "没有可清除的客户端注册",
      );
      invalidate();
    },
    onError: (error) => toast.fromError(error, "清除客户端注册失败"),
  });
  const reorder = useMutation<
    void,
    unknown,
    string[],
    { previous?: ServerSummary[] }
  >({
    mutationFn: (ids) => reorderServers(ids),
    onMutate: async (ids) => {
      await queryClient.cancelQueries({ queryKey: ["servers"] });
      const previous = queryClient.getQueryData<ServerSummary[]>(["servers"]);
      if (previous) {
        const byId = new Map(previous.map((server) => [server.id, server]));
        const ordered = ids
          .map((id) => byId.get(id))
          .filter((server): server is ServerSummary => Boolean(server));
        const remaining = previous.filter((server) => !ids.includes(server.id));
        queryClient.setQueryData(["servers"], [...ordered, ...remaining]);
      }
      return { previous };
    },
    onError: (error, _ids, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["servers"], context.previous);
      }
      toast.fromError(error, "保存服务器顺序失败");
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["servers"] });
    },
  });

  async function startEdit(id: string) {
    const config = await getServer(id);
    setEditing(config);
    setShowForm(true);
  }

  const serverList = servers.data ?? [];

  function resetDrag() {
    dragIdRef.current = null;
    setDragId(null);
    setDropTarget(null);
  }

  function saveOrder(ids: string[]) {
    if (reorder.isPending) return;
    const current = serverList.map((server) => server.id);
    if (
      ids.length === current.length &&
      ids.every((id, index) => id === current[index])
    ) {
      return;
    }
    reorder.mutate(ids);
  }

  function moveBy(id: string, direction: -1 | 1) {
    const ids = serverList.map((server) => server.id);
    const from = ids.indexOf(id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= ids.length) return;
    saveOrder(moveItem(ids, from, to));
  }

  function handleDragStart(
    event: DragEvent<HTMLButtonElement>,
    server: ServerSummary,
  ) {
    if (reorder.isPending) {
      event.preventDefault();
      return;
    }
    dragIdRef.current = server.id;
    setDragId(server.id);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", server.id);
  }

  function handleDragOver(
    event: DragEvent<HTMLDivElement>,
    server: ServerSummary,
  ) {
    const sourceId = dragIdRef.current;
    if (!sourceId || sourceId === server.id) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const rect = event.currentTarget.getBoundingClientRect();
    const after = event.clientY > rect.top + rect.height / 2;
    setDropTarget((current) =>
      current?.id === server.id && current.after === after
        ? current
        : { id: server.id, after },
    );
  }

  function handleDrop(event: DragEvent<HTMLDivElement>, server: ServerSummary) {
    event.preventDefault();
    const sourceId = dragIdRef.current;
    if (!sourceId || sourceId === server.id) {
      resetDrag();
      return;
    }
    const ids = serverList.map((item) => item.id);
    const from = ids.indexOf(sourceId);
    const target = ids.indexOf(server.id);
    if (from < 0 || target < 0) {
      resetDrag();
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    const after = event.clientY > rect.top + rect.height / 2;
    let insertAt = target + (after ? 1 : 0);
    if (from < insertAt) insertAt -= 1;
    const nextIds = moveItem(ids, from, insertAt);
    resetDrag();
    saveOrder(nextIds);
  }

  function handleKeyboard(event: KeyboardEvent<HTMLButtonElement>, id: string) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    moveBy(id, event.key === "ArrowUp" ? -1 : 1);
  }

  return (
    <section className="flex h-full flex-col">
      <div className="mb-2 flex items-center justify-between">
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

      {serverList.length > 1 ? (
        <p className="mb-3 text-[11px] text-slate-400">
          拖动左侧手柄调整顺序；接口/测试页的服务器选择列表会按状态分组，组内沿用此顺序。
        </p>
      ) : null}

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
          serverList.map((server) => {
            const ready = server.state.state === "ready";
            const busy =
              connect.isPending || disconnect.isPending || reconnect.isPending;
            const stateError =
              server.state.state === "error"
                ? toErrorInfo(server.state.error)
                : null;
            const oauthRequired =
              server.state.state === "error" &&
              server.state.error.kind === "oauth_required";
            const oauthConfigured = Boolean(server.oauth?.configured);
            const oauthAuthorized = Boolean(server.oauth?.authorized);
            const oauthStepUp = Boolean(server.oauth?.step_up_required);
            const oauthRequiredScope = server.oauth?.required_scope ?? null;
            const dropClass =
              dropTarget?.id === server.id
                ? dropTarget.after
                  ? "border-b-2 border-b-sky-500"
                  : "border-t-2 border-t-sky-500"
                : "";
            return (
              <div
                key={server.id}
                onDragOver={(event) => handleDragOver(event, server)}
                onDrop={(event) => handleDrop(event, server)}
                className={`flex items-center justify-between rounded-lg border border-slate-200 bg-white p-3 ${
                  dragId === server.id ? "opacity-50" : ""
                } ${dropClass}`}
              >
                <div className="flex min-w-0 items-center gap-2">
                  <button
                    type="button"
                    draggable={!reorder.isPending}
                    onDragStart={(event) => handleDragStart(event, server)}
                    onDragEnd={resetDrag}
                    onKeyDown={(event) => handleKeyboard(event, server.id)}
                    disabled={reorder.isPending}
                    className="shrink-0 cursor-grab rounded px-1 py-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600 active:cursor-grabbing disabled:cursor-not-allowed disabled:opacity-40"
                    aria-label={`调整「${server.name}」顺序，使用上下方向键移动`}
                    title="拖动排序；聚焦后可用 ↑/↓"
                  >
                    ⠿
                  </button>
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
                      {oauthConfigured ? (
                        <span
                          className={`rounded px-1.5 py-0.5 text-[11px] ${
                            oauthAuthorized && !oauthRequired && !oauthStepUp
                              ? "bg-emerald-50 text-emerald-600"
                              : "bg-amber-50 text-amber-700"
                          }`}
                          title={
                            server.oauth?.refreshable
                              ? `OAuth 已登录${server.oauth.expires_at ? `，过期时间 ${server.oauth.expires_at}` : ""}`
                              : "OAuth 本次会话有效；重启后需重新登录"
                          }
                        >
                          {oauthRequired
                            ? "OAuth 需要登录"
                            : oauthStepUp
                              ? "OAuth 需要补充授权"
                              : oauthAuthorized
                                ? "OAuth 已登录"
                                : "OAuth 未登录"}
                        </span>
                      ) : null}
                    </div>
                    <StatusBadge state={server.state} />
                    {oauthStepUp ? (
                      <p className="text-[11px] text-amber-700">
                        缺少权限：
                        {oauthRequiredScope ?? "由服务器挑战指定"}
                      </p>
                    ) : null}
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
                  {oauthRequired ||
                  oauthStepUp ||
                  (oauthConfigured && !oauthAuthorized) ? (
                    <button
                      type="button"
                      className="rounded border border-sky-200 bg-sky-50 px-2 py-1 text-sky-700 hover:bg-sky-100"
                      onClick={() => setOauthServer(server)}
                    >
                      {oauthStepUp ? "补充授权" : "登录"}
                    </button>
                  ) : null}
                  {oauthConfigured && oauthAuthorized && !oauthRequired ? (
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50 disabled:opacity-50"
                      onClick={() => logout.mutate(server.id)}
                      disabled={logout.isPending}
                    >
                      退出登录
                    </button>
                  ) : null}
                  {oauthConfigured ? (
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-1 hover:bg-slate-50"
                      onClick={() => setDiagnosticsServer(server)}
                    >
                      诊断
                    </button>
                  ) : null}
                  {oauthConfigured ? (
                    <button
                      type="button"
                      className="rounded border border-amber-200 px-2 py-1 text-amber-700 hover:bg-amber-50 disabled:opacity-50"
                      onClick={() => {
                        if (
                          window.confirm(
                            "清除客户端注册会删除当前 DCR/CIMD 身份和 token，下次登录将重新注册。继续？",
                          )
                        ) {
                          clearRegistration.mutate(server.id);
                        }
                      }}
                      disabled={clearRegistration.isPending}
                    >
                      清除注册
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

      {oauthServer ? (
        <OAuthLoginDialog
          serverId={oauthServer.id}
          serverName={oauthServer.name}
          onClose={() => setOauthServer(null)}
          onAuthorized={() => {
            const id = oauthServer.id;
            invalidate();
            reconnect.mutate(id);
          }}
        />
      ) : null}

      {diagnosticsServer ? (
        <OAuthDiagnosticsDialog
          serverId={diagnosticsServer.id}
          serverName={diagnosticsServer.name}
          onClose={() => setDiagnosticsServer(null)}
        />
      ) : null}

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
