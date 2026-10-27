import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useSearchParams } from "react-router-dom";
import { EmptyState } from "../components/EmptyState";
import { ExportPreviewDialog } from "../components/ExportPreviewDialog";
import { JsonViewer } from "../components/JsonViewer";
import { ResultViewer } from "../components/ResultViewer";
import { ServerSelect } from "../components/ServerSelect";
import { Field, Modal } from "../components/testing/SuiteDialog";
import { listServers } from "../api/servers";
import { listTestSuites, saveCallAsCase } from "../api/tests";
import {
  clearCallRecords,
  deleteCallRecords,
  exportCallRecords,
  getCallRecord,
  listCallRecords,
  previewExportCallRecords,
  type CallFilterInput,
} from "../api/history";
import { CALL_METHOD_LABEL } from "../lib/replay";
import { useReplayStore } from "../stores/replay";
import { toast } from "../stores/toast";
import type { CallRecord } from "../types/models";

const STATUS_LABEL: Record<string, { text: string; className: string }> = {
  ok: { text: "成功", className: "text-emerald-600" },
  error: { text: "失败", className: "text-red-600" },
  canceled: { text: "取消", className: "text-slate-500" },
};

/** 把 invoke 抛出的错误转成可展示文本 */
function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return String(error);
}

/** 历史页：筛选 / 列表 / 详情 / 载入重放 / 删除 / 导出 */
export function History() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const setReplay = useReplayStore((s) => s.setReplay);

  const [filter, setFilter] = useState<CallFilterInput>(() => {
    const originParam = searchParams.get("origin");
    const origin: CallFilterInput["origin"] =
      originParam === "all" ? null : originParam === "test" ? "test" : "manual";
    return {
      keyword_full: false,
      origin,
      server_id: searchParams.get("server_id"),
      method: searchParams.get("method"),
      target: searchParams.get("target"),
    };
  });
  const [keywordDraft, setKeywordDraft] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [includeFrames, setIncludeFrames] = useState(false);
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveSuiteId, setSaveSuiteId] = useState("");
  const [saveName, setSaveName] = useState("");

  const exportLoad = useCallback(
    () =>
      previewExportCallRecords(filter, includeFrames).then(
        (preview) => preview.json,
      ),
    [filter, includeFrames],
  );

  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const records = useQuery({
    queryKey: ["history", filter],
    queryFn: () => listCallRecords(filter, 0, 200),
  });
  const detail = useQuery({
    queryKey: ["history-detail", selected],
    queryFn: () => getCallRecord(selected ?? ""),
    enabled: Boolean(selected),
  });
  const saveSuites = useQuery({
    queryKey: ["test-suites", detail.data?.server_id],
    queryFn: () => listTestSuites(detail.data?.server_id ?? null),
    enabled: Boolean(saveOpen && detail.data),
  });
  const saveCase = useMutation({
    mutationFn: () =>
      saveCallAsCase(
        detail.data?.id ?? "",
        saveSuiteId,
        saveName.trim() || undefined,
      ),
    onSuccess: () => {
      toast.success("已保存为测试用例");
      setSaveOpen(false);
      setSaveName("");
    },
    onError: (error) => toast.fromError(error, "保存用例失败"),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["history"] });
    if (selected) {
      void queryClient.invalidateQueries({
        queryKey: ["history-detail", selected],
      });
    }
  };

  const remove = useMutation({
    mutationFn: (id: string) => deleteCallRecords([id]),
    onSuccess: () => {
      setSelected(null);
      invalidate();
    },
    onError: (error) => toast.fromError(error, "删除失败"),
  });

  const clear = useMutation({
    mutationFn: () => clearCallRecords(filter),
    onSuccess: (count) => {
      toast.success(`已清空 ${count} 条记录`);
      setSelected(null);
      invalidate();
    },
    onError: (error) => toast.fromError(error, "清空失败"),
  });

  const exportRecords = useMutation({
    mutationFn: (payload: { path: string; includeFrames: boolean }) =>
      exportCallRecords(filter, payload.path, payload.includeFrames),
    onError: (error) => toast.fromError(error, "导出失败"),
  });

  function removeTargetFromUrl() {
    if (!searchParams.has("target")) return;
    const next = new URLSearchParams(searchParams);
    next.delete("target");
    setSearchParams(next, { replace: true });
  }

  function clearTarget() {
    setFilter((previous) => ({ ...previous, target: null }));
    removeTargetFromUrl();
  }

  function replay(record: CallRecord) {
    if (record.method === "probe") {
      toast.error("探测记录不支持重放");
      return;
    }
    const serversData = servers.data ?? [];
    if (
      servers.isSuccess &&
      !serversData.some((s) => s.id === record.server_id)
    ) {
      toast.error("该记录所属服务器已删除，无法重放");
      return;
    }
    const server = serversData.find((s) => s.id === record.server_id);
    if (server && server.state.state !== "ready") {
      toast.error(`服务器「${server.name}」未连接，请先连接后再重放`);
      return;
    }
    setReplay({
      serverId: record.server_id,
      method: record.method,
      target: record.target,
      params: record.params,
    });
    navigate("/explorer");
  }

  return (
    <section className="flex h-full flex-col">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="text-xl font-semibold text-slate-800">历史</h2>
        <ServerSelect
          servers={servers.data ?? []}
          value={filter.server_id ?? ""}
          onChange={(id) => {
            setFilter({ ...filter, server_id: id || null, target: null });
            removeTargetFromUrl();
          }}
          placeholder="全部服务器"
          groupByStatus={false}
          allowDisabled
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          ariaLabel="按服务器筛选"
        />
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={filter.status ?? ""}
          onChange={(e) =>
            setFilter({ ...filter, status: e.target.value || null })
          }
        >
          <option value="">全部状态</option>
          <option value="ok">成功</option>
          <option value="error">失败</option>
          <option value="canceled">取消</option>
        </select>
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={filter.origin ?? "all"}
          onChange={(event) =>
            setFilter({
              ...filter,
              origin:
                event.target.value === "all"
                  ? null
                  : (event.target.value as "manual" | "test"),
            })
          }
        >
          <option value="manual">手工调用</option>
          <option value="test">测试调用</option>
          <option value="all">全部来源</option>
        </select>
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={filter.method ?? ""}
          onChange={(e) => {
            setFilter({
              ...filter,
              method: e.target.value || null,
              target: null,
            });
            removeTargetFromUrl();
          }}
        >
          <option value="">全部方法</option>
          {Object.entries(CALL_METHOD_LABEL).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <input
          className="w-48 rounded border border-slate-200 px-2 py-1 text-xs"
          placeholder="关键词（target/服务器）"
          value={keywordDraft}
          onChange={(e) => setKeywordDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              setFilter({ ...filter, keyword: keywordDraft || null });
            }
          }}
        />
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
          onClick={() =>
            setFilter({ ...filter, keyword: keywordDraft || null })
          }
        >
          搜索
        </button>
        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50"
            onClick={() => setExportOpen(true)}
          >
            导出 JSON
          </button>
          <button
            type="button"
            className="rounded border border-red-100 px-2 py-1 text-xs text-red-500 hover:bg-red-50"
            onClick={() => {
              const scope = filter.target
                ? `接口「${filter.target}」的`
                : "当前筛选条件下的";
              if (window.confirm(`确定清空${scope}全部历史记录？`)) {
                clear.mutate();
              }
            }}
          >
            清空
          </button>
        </div>
      </div>

      {filter.target ? (
        <div className="mb-2 flex items-center gap-2 text-[11px] text-slate-500">
          <span className="inline-flex items-center gap-1 rounded bg-slate-100 px-2 py-0.5 text-slate-600">
            接口：
            <span className="max-w-80 truncate font-mono">{filter.target}</span>
            <button
              type="button"
              className="ml-1 rounded px-1 text-slate-400 hover:bg-slate-200 hover:text-slate-700"
              aria-label="清除接口筛选"
              onClick={clearTarget}
            >
              ×
            </button>
          </span>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 gap-4">
        <div className="w-96 shrink-0 overflow-y-auto rounded-lg border border-slate-200 bg-white">
          {records.isLoading ? (
            <div className="p-3 text-xs text-slate-400">加载中…</div>
          ) : (records.data?.items ?? []).length === 0 ? (
            <div className="p-3 text-xs text-slate-400">
              暂无记录（调用接口后自动记录）
            </div>
          ) : (
            (records.data?.items ?? []).map((record) => (
              <button
                key={record.id}
                type="button"
                className={`block w-full border-b border-slate-100 px-3 py-2 text-left text-xs last:border-b-0 ${
                  selected === record.id ? "bg-slate-100" : "hover:bg-slate-50"
                }`}
                onClick={() => setSelected(record.id)}
              >
                <div className="flex items-center gap-2">
                  <span className="rounded bg-slate-100 px-1 py-0.5 text-[10px] text-slate-500">
                    {CALL_METHOD_LABEL[record.method]}
                  </span>
                  <span className="truncate text-slate-800">
                    {record.target}
                  </span>
                  <span
                    className={`ml-auto ${STATUS_LABEL[record.status]?.className ?? ""}`}
                  >
                    {STATUS_LABEL[record.status]?.text ?? record.status}
                  </span>
                </div>
                <div className="mt-0.5 text-slate-400">
                  {record.server_name} · {record.duration_ms}ms ·{" "}
                  {new Date(record.created_at).toLocaleString()}
                  {record.truncated ? " · 结果已截断" : ""}
                </div>
              </button>
            ))
          )}
        </div>

        <div className="min-w-0 flex-1 overflow-y-auto">
          {!selected ? (
            <EmptyState
              title="选择一条记录"
              description="查看详情、载入重放或删除。"
            />
          ) : detail.isPending ? (
            <div className="text-sm text-slate-400">加载详情中…</div>
          ) : detail.isError ? (
            <div className="space-y-2">
              <div className="text-sm text-red-500">详情加载失败</div>
              <div className="break-all text-xs text-slate-500">
                {errorText(detail.error)}
              </div>
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50 disabled:opacity-50"
                disabled={detail.isFetching}
                onClick={() => void detail.refetch()}
              >
                {detail.isFetching ? "重试中…" : "重试"}
              </button>
            </div>
          ) : detail.data ? (
            <div className="space-y-3">
              <div className="flex items-center gap-2 text-xs">
                <button
                  type="button"
                  className="rounded bg-slate-800 px-3 py-1.5 text-white hover:bg-slate-700"
                  onClick={() => {
                    if (detail.data) replay(detail.data);
                  }}
                >
                  载入调用台重放
                </button>
                <button
                  type="button"
                  className="rounded border border-slate-200 px-3 py-1.5 hover:bg-slate-50 disabled:opacity-50"
                  disabled={detail.data.method === "probe"}
                  title={
                    detail.data.method === "probe"
                      ? "探测记录不支持保存为用例"
                      : "保存到测试套件"
                  }
                  onClick={() => {
                    setSaveSuiteId("");
                    setSaveName(`${detail.data?.target ?? ""} · 来自历史`);
                    setSaveOpen(true);
                  }}
                >
                  保存为用例
                </button>
                <button
                  type="button"
                  className="rounded border border-red-100 px-3 py-1.5 text-red-500 hover:bg-red-50"
                  onClick={() => {
                    if (selected) remove.mutate(selected);
                  }}
                >
                  删除
                </button>
                <span className="text-slate-400">
                  {detail.data.server_name} ·{" "}
                  {CALL_METHOD_LABEL[detail.data.method]} ·{" "}
                  {new Date(detail.data.created_at).toLocaleString()}
                </span>
              </div>
              <details className="rounded-lg border border-slate-200 bg-white p-3">
                <summary className="cursor-pointer text-xs text-slate-500">
                  请求参数
                </summary>
                <div className="mt-2">
                  <JsonViewer
                    value={detail.data.params}
                    defaultMode="tree"
                    copyLabel="复制参数"
                    contentClassName="max-h-48 overflow-auto"
                  />
                </div>
              </details>
              <ResultViewer record={detail.data} />
            </div>
          ) : (
            <div className="text-sm text-slate-400">暂无详情</div>
          )}
        </div>
      </div>

      {saveOpen && detail.data ? (
        <Modal title="保存为测试用例" onClose={() => setSaveOpen(false)}>
          <div className="space-y-3 text-sm">
            <Field label="目标套件">
              <select
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={saveSuiteId}
                onChange={(event) => setSaveSuiteId(event.target.value)}
              >
                <option value="">请选择套件</option>
                {(saveSuites.data ?? []).map((suite) => (
                  <option key={suite.id} value={suite.id}>
                    {suite.name}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="用例名称">
              <input
                className="w-full rounded border border-slate-200 px-2 py-1.5"
                value={saveName}
                onChange={(event) => setSaveName(event.target.value)}
              />
            </Field>
            <p className="text-[11px] text-slate-400">
              仅保存参数与基础期望；可在“测试”页继续补充断言。
            </p>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
                onClick={() => setSaveOpen(false)}
              >
                取消
              </button>
              <button
                type="button"
                className="rounded bg-slate-800 px-3 py-1.5 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
                disabled={!saveSuiteId || saveCase.isPending}
                onClick={() => saveCase.mutate()}
              >
                {saveCase.isPending ? "保存中…" : "保存"}
              </button>
            </div>
          </div>
        </Modal>
      ) : null}

      {exportOpen ? (
        <ExportPreviewDialog
          key={includeFrames ? "frames" : "default"}
          title="导出历史记录"
          defaultFilename={`mcptester-history-${new Date()
            .toISOString()
            .slice(0, 16)
            .replace(/[^0-9]/g, "")}.json`}
          load={exportLoad}
          onSave={(path) => exportRecords.mutateAsync({ path, includeFrames })}
          onClose={() => {
            setExportOpen(false);
            setIncludeFrames(false);
          }}
          extra={
            <label className="flex items-center gap-1 text-[11px] text-slate-500">
              <input
                type="checkbox"
                checked={includeFrames}
                onChange={(event) => setIncludeFrames(event.target.checked)}
              />
              包含原始帧（体积会明显增大，上限 50MB）
            </label>
          }
        />
      ) : null}
    </section>
  );
}
