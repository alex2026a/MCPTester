import { useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSearchParams } from "react-router-dom";
import { EmptyState } from "../components/EmptyState";
import { ExportPreviewDialog } from "../components/ExportPreviewDialog";
import { CaseEditor } from "../components/testing/CaseEditor";
import { GenerationDialog } from "../components/testing/GenerationDialog";
import { LlmDialog } from "../components/testing/LlmDialog";
import { RunDialog } from "../components/testing/RunDialog";
import { RunProgress } from "../components/testing/RunProgress";
import { SuiteDialog } from "../components/testing/SuiteDialog";
import {
  deleteTestCases,
  deleteTestSuite,
  diffContract,
  duplicateTestCase,
  exportTestSuite,
  getActiveTestRun,
  importTestCases,
  listTestCases,
  listTestSuites,
  reorderTestCases,
} from "../api/tests";
import { listServers } from "../api/servers";
import { toast } from "../stores/toast";
import type { TestCase, TestSuiteSummary } from "../types/testing";

const RISK_LABEL: Record<string, string> = {
  read_only: "只读",
  write: "写",
  unknown: "未知",
  destructive: "破坏性",
};

const PRIORITY_LABEL: Record<string, string> = {
  p0: "P0",
  p1: "P1",
  p2: "P2",
};

/** 测试用例库（M1）：套件/用例 CRUD、排序、批量删除、导入导出、契约漂移。 */
export function Tests() {
  const queryClient = useQueryClient();
  const [searchParams] = useSearchParams();
  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const [serverId, setServerId] = useState("");
  const [suiteId, setSuiteId] = useState("");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [suiteDialog, setSuiteDialog] = useState<
    { suite?: TestSuiteSummary } | null
  >(null);
  const [editor, setEditor] = useState<TestCase | "new" | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [generateOpen, setGenerateOpen] = useState(
    () => Boolean(searchParams.get("target")),
  );
  const [generateTarget, setGenerateTarget] = useState<string | null>(() =>
    searchParams.get("target"),
  );
  const [runDialog, setRunDialog] = useState<"suite" | "selected" | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [llmOpen, setLlmOpen] = useState(false);
  const [contractDiff, setContractDiff] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const serverList = servers.data ?? [];
  const activeServerId = serverId || serverList[0]?.id || "";
  const activeServer = serverList.find((server) => server.id === activeServerId);

  const suites = useQuery({
    queryKey: ["test-suites", activeServerId],
    queryFn: () => listTestSuites(activeServerId || null),
    enabled: Boolean(activeServerId),
  });
  const suiteList = suites.data ?? [];
  const activeSuite =
    suiteList.find((suite) => suite.id === suiteId) ?? suiteList[0];

  const cases = useQuery({
    queryKey: ["test-cases", activeSuite?.id],
    queryFn: () => listTestCases(activeSuite?.id ?? ""),
    enabled: Boolean(activeSuite?.id),
  });
  const activeRun = useQuery({
    queryKey: ["active-test-run"],
    queryFn: getActiveTestRun,
    refetchInterval: 5000,
  });
  const caseList = cases.data ?? [];
  const selectedSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedCases = caseList.filter((item) => selectedSet.has(item.id));

  const invalidateCases = () => {
    void queryClient.invalidateQueries({ queryKey: ["test-cases"] });
    void queryClient.invalidateQueries({ queryKey: ["test-suites"] });
  };

  const removeCases = useMutation({
    mutationFn: (ids: string[]) => deleteTestCases(ids),
    onSuccess: (count) => {
      toast.success(`已删除 ${count} 条用例`);
      setSelectedIds([]);
      invalidateCases();
    },
    onError: (error) => toast.fromError(error, "删除用例失败"),
  });

  const removeSuite = useMutation({
    mutationFn: (id: string) => deleteTestSuite(id),
    onSuccess: () => {
      toast.success("套件已删除");
      setSuiteId("");
      setSelectedIds([]);
      invalidateCases();
    },
    onError: (error) => toast.fromError(error, "删除套件失败"),
  });

  const duplicate = useMutation({
    mutationFn: (id: string) => duplicateTestCase(id),
    onSuccess: () => {
      toast.success("已复制用例");
      invalidateCases();
    },
    onError: (error) => toast.fromError(error, "复制用例失败"),
  });

  const move = useMutation({
    mutationFn: ({ ids }: { ids: string[] }) =>
      reorderTestCases(activeSuite?.id ?? "", ids),
    onSuccess: invalidateCases,
    onError: (error) => toast.fromError(error, "排序失败"),
  });

  const importCases = useMutation({
    mutationFn: (json: string) =>
      importTestCases(activeSuite?.id ?? "", json, "import"),
    onSuccess: (report) => {
      toast.success(`已导入 ${report.added} 条用例`, {
        message:
          report.failed.length > 0
            ? `${report.failed.length} 条失败，请查看控制台`
            : undefined,
      });
      if (report.failed.length > 0) {
        console.warn("导入失败明细", report.failed);
      }
      invalidateCases();
    },
    onError: (error) => toast.fromError(error, "导入失败"),
  });

  const compareContract = useMutation({
    mutationFn: () => diffContract(activeSuite?.id ?? ""),
    onSuccess: (diff) => {
      if (!diff.changed) {
        setContractDiff("契约未变化");
        return;
      }
      const parts = [
        `新增 ${diff.added.length}`,
        `删除 ${diff.removed.length}`,
        `修改 ${diff.modified.length}`,
      ];
      setContractDiff(parts.join(" · "));
    },
    onError: (error) => toast.fromError(error, "契约对比失败"),
  });

  function toggle(id: string) {
    setSelectedIds((current) =>
      current.includes(id)
        ? current.filter((item) => item !== id)
        : [...current, id],
    );
  }

  function toggleAll() {
    setSelectedIds((current) =>
      current.length === caseList.length ? [] : caseList.map((item) => item.id),
    );
  }

  function moveCase(id: string, direction: -1 | 1) {
    if (!activeSuite) return;
    const index = caseList.findIndex((item) => item.id === id);
    const next = index + direction;
    if (index < 0 || next < 0 || next >= caseList.length) return;
    const ids = caseList.map((item) => item.id);
    [ids[index], ids[next]] = [ids[next], ids[index]];
    move.mutate({ ids });
  }

  return (
    <section className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-xl font-semibold text-slate-800">测试用例</h2>
        <select
          className="rounded border border-slate-200 px-2 py-1 text-xs"
          value={activeServerId}
          onChange={(event) => {
            setServerId(event.target.value);
            setSuiteId("");
            setSelectedIds([]);
            setContractDiff(null);
          }}
        >
          <option value="">选择服务器</option>
          {serverList.map((server) => (
            <option key={server.id} value={server.id}>
              {server.name}
            </option>
          ))}
        </select>
        {activeServer ? (
          <span className="text-[11px] text-slate-400">
            状态：{activeServer.state.state}
          </span>
        ) : null}
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeServerId}
            onClick={() => setSuiteDialog({})}
          >
            新建套件
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeSuite}
            onClick={() => fileInputRef.current?.click()}
          >
            导入
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeSuite}
            onClick={() => setExportOpen(true)}
          >
            导出套件
          </button>
          <button
            type="button"
            className="rounded bg-slate-800 px-2 py-1 text-xs text-white hover:bg-slate-700 disabled:opacity-50"
            disabled={!activeSuite}
            onClick={() => setEditor("new")}
          >
            新建用例
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeSuite}
            onClick={() => {
              setGenerateTarget(null);
              setGenerateOpen(true);
            }}
          >
            生成用例
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeSuite}
            onClick={() => setLlmOpen(true)}
          >
            LLM 辅助
          </button>

          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={!activeSuite || Boolean(runId || activeRun.data)}
            onClick={() => setRunDialog("suite")}
          >
            运行套件
          </button>
          <button
            type="button"
            className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-50"
            disabled={
              selectedCases.length === 0 || Boolean(runId || activeRun.data)
            }
            onClick={() => setRunDialog("selected")}
          >
            运行选中
          </button>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={async (event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (!file || !activeSuite) return;
            const text = await file.text();
            importCases.mutate(text);
          }}
        />
      </div>

      {runId || activeRun.data ? (
        <RunProgress
          runId={runId ?? activeRun.data?.run_id ?? ""}
          onClose={() => {
            setRunId(null);
            void queryClient.invalidateQueries({ queryKey: ["active-test-run"] });
          }}
          onFinished={() => {
            setRunId(null);
            void queryClient.invalidateQueries({ queryKey: ["active-test-run"] });
            invalidateCases();
          }}
        />
      ) : null}

      {!activeServerId ? (
        <EmptyState
          title="先选择服务器"
          description="测试套件必须绑定一个 MCP 服务器。"
        />
      ) : (
        <div className="flex min-h-0 flex-1 gap-3">
          <aside className="w-64 shrink-0 overflow-y-auto rounded-lg border border-slate-200 bg-white p-2">
            <div className="mb-2 flex items-center justify-between px-1">
              <span className="text-xs font-medium text-slate-500">套件</span>
              <span className="text-[11px] text-slate-400">
                {suiteList.length}
              </span>
            </div>
            {suites.isLoading ? (
              <div className="p-2 text-xs text-slate-400">加载中…</div>
            ) : suiteList.length === 0 ? (
              <div className="p-2 text-xs text-slate-400">
                暂无套件，点击“新建套件”开始。
              </div>
            ) : (
              suiteList.map((suite) => (
                <button
                  key={suite.id}
                  type="button"
                  className={`mb-1 block w-full rounded px-2 py-2 text-left text-xs ${
                    activeSuite?.id === suite.id
                      ? "bg-slate-800 text-white"
                      : "hover:bg-slate-100"
                  }`}
                  onClick={() => {
                    setSuiteId(suite.id);
                    setSelectedIds([]);
                    setContractDiff(null);
                  }}
                >
                  <div className="flex items-center gap-2">
                    <span className="truncate">{suite.name}</span>
                    <span
                      className={`ml-auto ${
                        activeSuite?.id === suite.id
                          ? "text-slate-300"
                          : "text-slate-400"
                      }`}
                    >
                      {suite.enabled_count}/{suite.case_count}
                    </span>
                  </div>
                  {suite.last_run_status ? (
                    <div className="mt-0.5 text-[10px] opacity-70">
                      最近：{suite.last_run_status}
                    </div>
                  ) : null}
                </button>
              ))
            )}
            {activeSuite ? (
              <div className="mt-2 flex flex-wrap gap-1 border-t border-slate-100 pt-2">
                <button
                  type="button"
                  className="rounded border border-slate-200 px-1.5 py-1 text-[11px] hover:bg-slate-50"
                  onClick={() => setSuiteDialog({ suite: activeSuite })}
                >
                  编辑
                </button>
                <button
                  type="button"
                  className="rounded border border-red-100 px-1.5 py-1 text-[11px] text-red-500 hover:bg-red-50"
                  onClick={() => {
                    if (window.confirm(`删除套件「${activeSuite.name}」及其全部用例？`)) {
                      removeSuite.mutate(activeSuite.id);
                    }
                  }}
                >
                  删除
                </button>
                <button
                  type="button"
                  className="rounded border border-slate-200 px-1.5 py-1 text-[11px] hover:bg-slate-50"
                  onClick={() => compareContract.mutate()}
                >
                  契约漂移
                </button>
              </div>
            ) : null}
            {contractDiff ? (
              <div className="mt-2 rounded bg-slate-50 p-2 text-[11px] text-slate-500">
                {contractDiff}
              </div>
            ) : null}
          </aside>

          <div className="min-w-0 flex-1 overflow-y-auto rounded-lg border border-slate-200 bg-white">
            {!activeSuite ? (
              <EmptyState
                title="暂无套件"
                description="新建套件后即可添加/导入用例。"
              />
            ) : cases.isLoading ? (
              <div className="p-3 text-xs text-slate-400">加载用例中…</div>
            ) : caseList.length === 0 ? (
              <EmptyState
                title="套件还没有用例"
                description="新建用例，或从历史调用保存为用例。"
              />
            ) : (
              <>
                <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 p-2 text-xs">
                  <label className="flex items-center gap-1">
                    <input
                      type="checkbox"
                      checked={
                        caseList.length > 0 &&
                        selectedIds.length === caseList.length
                      }
                      onChange={toggleAll}
                    />
                    全选
                  </label>
                  <span className="text-slate-400">
                    已选 {selectedIds.length} / {caseList.length}
                  </span>
                  <button
                    type="button"
                    className="rounded border border-red-100 px-2 py-1 text-red-500 hover:bg-red-50 disabled:opacity-50"
                    disabled={selectedCases.length === 0}
                    onClick={() => {
                      if (window.confirm(`删除选中的 ${selectedCases.length} 条用例？`)) {
                        removeCases.mutate(selectedCases.map((item) => item.id));
                      }
                    }}
                  >
                    删除选中
                  </button>
                </div>
                <table className="w-full text-left text-xs">
                  <thead className="bg-slate-50 text-slate-500">
                    <tr>
                      <th className="w-8 px-2 py-2" />
                      <th className="px-2 py-2">名称</th>
                      <th className="px-2 py-2">方法 / 目标</th>
                      <th className="px-2 py-2">标签</th>
                      <th className="px-2 py-2">风险</th>
                      <th className="px-2 py-2">优先级</th>
                      <th className="px-2 py-2">启用</th>
                      <th className="w-40 px-2 py-2">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {caseList.map((item) => (
                      <tr key={item.id} className="border-t border-slate-100">
                        <td className="px-2 py-2">
                          <input
                            type="checkbox"
                            checked={selectedSet.has(item.id)}
                            onChange={() => toggle(item.id)}
                          />
                        </td>
                        <td className="max-w-64 px-2 py-2">
                          <button
                            type="button"
                            className="truncate text-left font-medium text-slate-800 hover:underline"
                            onClick={() => setEditor(item)}
                          >
                            {item.name}
                          </button>
                          {item.generation_note ? (
                            <div className="truncate text-[10px] text-slate-400">
                              {item.generation_note}
                            </div>
                          ) : null}
                        </td>
                        <td className="px-2 py-2 text-slate-500">
                          <div>{item.method}</div>
                          <div className="max-w-48 truncate">{item.target}</div>
                        </td>
                        <td className="px-2 py-2 text-slate-500">
                          {item.tags.join(", ")}
                        </td>
                        <td className="px-2 py-2">
                          <span
                            className={
                              item.risk === "destructive"
                                ? "text-red-600"
                                : item.risk === "unknown"
                                  ? "text-amber-600"
                                  : "text-slate-500"
                            }
                          >
                            {RISK_LABEL[item.risk] ?? item.risk}
                          </span>
                        </td>
                        <td className="px-2 py-2 text-slate-500">
                          {PRIORITY_LABEL[item.priority] ?? item.priority}
                        </td>
                        <td className="px-2 py-2 text-slate-500">
                          {item.enabled ? "是" : "否"}
                        </td>
                        <td className="px-2 py-2">
                          <div className="flex flex-wrap gap-1">
                            <button
                              type="button"
                              className="rounded border border-slate-200 px-1.5 py-0.5 hover:bg-slate-50"
                              onClick={() => setEditor(item)}
                            >
                              编辑
                            </button>
                            <button
                              type="button"
                              className="rounded border border-slate-200 px-1.5 py-0.5 hover:bg-slate-50"
                              onClick={() => duplicate.mutate(item.id)}
                            >
                              复制
                            </button>
                            <button
                              type="button"
                              className="rounded border border-slate-200 px-1.5 py-0.5 hover:bg-slate-50"
                              onClick={() => moveCase(item.id, -1)}
                            >
                              ↑
                            </button>
                            <button
                              type="button"
                              className="rounded border border-slate-200 px-1.5 py-0.5 hover:bg-slate-50"
                              onClick={() => moveCase(item.id, 1)}
                            >
                              ↓
                            </button>
                            <button
                              type="button"
                              className="rounded border border-red-100 px-1.5 py-0.5 text-red-500 hover:bg-red-50"
                              onClick={() => removeCases.mutate([item.id])}
                            >
                              删除
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            )}
          </div>
        </div>
      )}

      {suiteDialog ? (
        <SuiteDialog
          servers={serverList}
          suite={suiteDialog.suite}
          defaultServerId={activeServerId}
          onClose={() => setSuiteDialog(null)}
          onSaved={(id) => {
            setSuiteId(id);
            void queryClient.invalidateQueries({ queryKey: ["test-suites"] });
          }}
        />
      ) : null}

      {editor ? (
        <CaseEditor
          key={editor === "new" ? "new" : editor.id}
          suiteId={activeSuite?.id ?? ""}
          testCase={editor === "new" ? undefined : editor}
          onClose={() => setEditor(null)}
          onSaved={invalidateCases}
        />
      ) : null}

      {llmOpen && activeSuite && activeServerId ? (
        <LlmDialog
          suiteId={activeSuite.id}
          serverId={activeServerId}
          onClose={() => setLlmOpen(false)}
          onSaved={invalidateCases}
        />
      ) : null}

      {runDialog && activeSuite && activeServerId ? (
        <RunDialog
          mode={runDialog}
          suiteId={activeSuite.id}
          serverId={activeServerId}
          caseIds={selectedCases.map((item) => item.id)}
          caseCount={selectedCases.length}
          onClose={() => setRunDialog(null)}
          onStarted={(id) => {
            setRunId(id);
            void queryClient.invalidateQueries({ queryKey: ["active-test-run"] });
          }}
        />
      ) : null}

      {generateOpen && activeSuite && activeServerId ? (
        <GenerationDialog
          suiteId={activeSuite.id}
          serverId={activeServerId}
          target={generateTarget}
          onClose={() => {
            setGenerateOpen(false);
            setGenerateTarget(null);
          }}
          onSaved={invalidateCases}
        />
      ) : null}

      {exportOpen && activeSuite ? (
        <ExportPreviewDialog
          title="导出测试套件"
          defaultFilename={`mcptester-suite-${activeSuite.name}.json`}
          load={() => exportTestSuite(activeSuite.id)}
          onSave={(path) => exportTestSuite(activeSuite.id, path)}
          onClose={() => setExportOpen(false)}
        />
      ) : null}
    </section>
  );
}
