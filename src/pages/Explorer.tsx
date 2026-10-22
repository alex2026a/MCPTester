import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { EventPanel } from "../components/EventPanel";
import { JsonViewer } from "../components/JsonViewer";
import { ResultViewer } from "../components/ResultViewer";
import { SchemaForm, type SchemaFormHandle } from "../components/SchemaForm";
import {
  getCapabilities,
  listServers,
  refreshCapabilities,
} from "../api/servers";
import { getAppInfo } from "../api/app";
import { callTool, cancelCall, getPrompt, readResource } from "../api/calls";
import {
  CALL_METHOD_LABEL,
  extractTemplateVariables,
  matchUriTemplate,
  replayArguments,
  replayTab,
} from "../lib/replay";
import { useReplayStore } from "../stores/replay";
import type { CallRecord, CapabilitySnapshot } from "../types/models";

type Tab = "tools" | "resources" | "templates" | "prompts";

const TABS: { key: Tab; label: string }[] = [
  { key: "tools", label: "Tools" },
  { key: "resources", label: "Resources" },
  { key: "templates", label: "资源模板" },
  { key: "prompts", label: "Prompts" },
];

/** 参数是否为空（用于模板变量必填提示） */
function isBlank(value: string | undefined): boolean {
  return !(value ?? "").trim();
}

/** 接口页：枚举 + 调用台（Tools / Resources / 模板 / Prompts） */
export function Explorer() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const appInfo = useQuery({ queryKey: ["appInfo"], queryFn: getAppInfo });
  const replay = useReplayStore((s) => s.replay);
  const setReplay = useReplayStore((s) => s.setReplay);

  // 历史重放：首帧直接用 store 初始化，避免先渲染默认服务器/空选中再跳变
  const [serverId, setServerId] = useState<string>(
    () => replay?.serverId ?? "",
  );
  const [tab, setTab] = useState<Tab>(() =>
    replay ? (replayTab(replay.method) ?? "tools") : "tools",
  );
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string>(() => replay?.target ?? "");
  const [result, setResult] = useState<CallRecord | null>(null);
  const [inFlightId, setInFlightId] = useState<string | null>(null);
  const [promptArgs, setPromptArgs] = useState<Record<string, unknown>>(() =>
    replay?.method === "prompts_get" ? replayArguments(replay.params) : {},
  );
  const [templateVars, setTemplateVars] = useState<Record<string, string>>({});
  const [toolArgs, setToolArgs] = useState<Record<string, unknown>>(() =>
    replay?.method === "tools_call" ? replayArguments(replay.params) : {},
  );
  const [formNonce, setFormNonce] = useState(0);
  const [toolForceSend, setToolForceSend] = useState(false);
  const [promptSubmitted, setPromptSubmitted] = useState(false);
  const [promptForceSend, setPromptForceSend] = useState(false);
  const [replayBanner, setReplayBanner] = useState<string | null>(() =>
    replay
      ? `已载入历史重放：${CALL_METHOD_LABEL[replay.method]} · ${replay.target}`
      : null,
  );
  const [pendingResource, setPendingResource] = useState<string | null>(() =>
    replay?.method === "resources_read" ? replay.target : null,
  );
  const selectedItemRef = useRef<HTMLButtonElement | null>(null);
  const toolFormRef = useRef<SchemaFormHandle | null>(null);

  const currentServer = serverId
    ? (servers.data ?? []).find((s) => s.id === serverId)
    : appInfo.data?.default_server_id
      ? ((servers.data ?? []).find(
          (s) => s.id === appInfo.data?.default_server_id,
        ) ?? (servers.data ?? [])[0])
      : (servers.data ?? [])[0];
  const currentServerId = currentServer?.id;
  const missingServer =
    Boolean(serverId) && servers.isSuccess && !currentServer;
  const ready = currentServer?.state.state === "ready";

  const capabilities = useQuery<CapabilitySnapshot>({
    queryKey: ["capabilities", currentServer?.id],
    queryFn: () => getCapabilities(currentServer?.id ?? ""),
    enabled: Boolean(currentServer?.id) && ready,
  });

  // 重放值已在首帧由 store 初始化；这里只清空一次性标记，避免返回接口页时重复应用
  useEffect(() => {
    if (replay) setReplay(null);
  }, [replay, setReplay]);

  // 启动配置指定的默认服务器由 currentServer 派生，不写入本地 state，
  // 避免 effect 内同步 setState；用户选择后 serverId 优先生效。

  // resources/read 可能是模板生成的 URI：能力快照到位后再派生具体资源/模板视图
  const resourceReplay = useMemo(() => {
    if (!pendingResource || !capabilities.data) return null;
    if (capabilities.data.server_id !== currentServerId) return null;

    const exact = capabilities.data.resources.some(
      (resource) => resource.uri === pendingResource,
    );
    if (exact) {
      return { tab: "resources" as Tab, selected: pendingResource, vars: {} };
    }

    const matched = capabilities.data.resource_templates
      .map((template) => ({
        template,
        vars: matchUriTemplate(template.uri_template, pendingResource),
      }))
      .find((item) => item.vars !== null);

    if (matched?.vars) {
      return {
        tab: "templates" as Tab,
        selected: matched.template.uri_template,
        vars: matched.vars,
      };
    }
    return { tab: "resources" as Tab, selected: pendingResource, vars: {} };
  }, [pendingResource, capabilities.data, currentServerId]);

  const viewTab = resourceReplay?.tab ?? tab;
  const viewSelected = resourceReplay?.selected ?? selected;
  const viewTemplateVars = resourceReplay
    ? { ...resourceReplay.vars, ...templateVars }
    : templateVars;

  // 重放后把左侧选中项滚动到可见区域
  useEffect(() => {
    if (!viewSelected) return;
    selectedItemRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [viewSelected, viewTab, capabilities.data]);

  const refresh = useMutation({
    mutationFn: () => refreshCapabilities(currentServer?.id ?? ""),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(["capabilities", currentServer?.id], snapshot);
    },
  });

  const sendTool = useMutation({
    mutationFn: async (payload: {
      name: string;
      args: Record<string, unknown>;
    }) => {
      const callId = crypto.randomUUID();
      setInFlightId(callId);
      try {
        return await callTool(
          currentServer?.id ?? "",
          callId,
          payload.name,
          payload.args,
        );
      } finally {
        setInFlightId(null);
      }
    },
    onSuccess: (record) => setResult(record),
  });

  const readRes = useMutation({
    mutationFn: (uri: string) => readResource(currentServer?.id ?? "", uri),
    onSuccess: (record) => setResult(record),
  });

  const fetchPrompt = useMutation({
    mutationFn: (payload: { name: string; args: Record<string, unknown> }) =>
      getPrompt(currentServer?.id ?? "", payload.name, payload.args),
    onSuccess: (record) => setResult(record),
  });

  const items = useMemo(() => {
    const snapshot = capabilities.data;
    if (!snapshot)
      return [] as { key: string; title: string; subtitle: string }[];
    const keyword = filter.trim().toLowerCase();
    const match = (text: string) =>
      !keyword || text.toLowerCase().includes(keyword);
    switch (viewTab) {
      case "tools":
        return snapshot.tools
          .filter((tool) => match(tool.name) || match(tool.description ?? ""))
          .map((tool) => ({
            key: tool.name,
            title: tool.name,
            subtitle: tool.description ?? "",
          }));
      case "resources":
        return snapshot.resources
          .filter((res) => match(res.uri) || match(res.name))
          .map((res) => ({ key: res.uri, title: res.name, subtitle: res.uri }));
      case "templates":
        return snapshot.resource_templates
          .filter((tpl) => match(tpl.name) || match(tpl.uri_template))
          .map((tpl) => ({
            key: tpl.uri_template,
            title: tpl.name,
            subtitle: tpl.uri_template,
          }));
      case "prompts":
        return snapshot.prompts
          .filter((prompt) => match(prompt.name))
          .map((prompt) => ({
            key: prompt.name,
            title: prompt.name,
            subtitle: prompt.description ?? "",
          }));
      default:
        return [];
    }
  }, [capabilities.data, filter, viewTab]);

  const currentTool = (capabilities.data?.tools ?? []).find(
    (t) => t.name === viewSelected,
  );
  const currentPrompt = (capabilities.data?.prompts ?? []).find(
    (p) => p.name === viewSelected,
  );
  const currentTemplate = (capabilities.data?.resource_templates ?? []).find(
    (t) => t.uri_template === viewSelected,
  );
  const templateNames = currentTemplate
    ? extractTemplateVariables(currentTemplate.uri_template)
    : [];
  const missingTemplateVars = templateNames.filter((name) =>
    isBlank(viewTemplateVars[name]),
  );
  const resourceKnown = (capabilities.data?.resources ?? []).some(
    (resource) => resource.uri === viewSelected,
  );
  const missingPromptArgs = (currentPrompt?.arguments ?? [])
    .filter(
      (arg) => arg.required && isBlank(String(promptArgs[arg.name] ?? "")),
    )
    .map((arg) => arg.name);

  function resetCallState() {
    setSelected("");
    setResult(null);
    setToolArgs({});
    setPromptArgs({});
    setTemplateVars({});
    setPendingResource(null);
    setToolForceSend(false);
    setPromptSubmitted(false);
    setPromptForceSend(false);
    setFormNonce((value) => value + 1);
  }

  function handleToolArgsChange(next: Record<string, unknown>) {
    setToolArgs(next);
    setToolForceSend(false);
  }

  function submitTool(force: boolean) {
    if (!currentTool) return;
    const form = toolFormRef.current;
    if (!form) return;

    if (!force) {
      const result = form.validate();
      // JSON 解析失败时无法确定要发送什么，不提供「仍要发送」
      if (result.parseError) {
        setToolForceSend(false);
        return;
      }
      if (result.errors.length > 0) {
        setToolForceSend(true);
        return;
      }
      setToolForceSend(false);
      sendTool.mutate({ name: currentTool.name, args: result.value });
      return;
    }

    const result = form.readValue();
    // JSON 解析失败时无法确定要发送什么，退回到展示错误
    if (result.parseError) {
      form.validate();
      return;
    }
    setToolForceSend(false);
    sendTool.mutate({ name: currentTool.name, args: result.value });
  }

  function submitPrompt(force: boolean) {
    if (!currentPrompt) return;
    if (!force && missingPromptArgs.length > 0) {
      setPromptSubmitted(true);
      setPromptForceSend(true);
      return;
    }
    setPromptSubmitted(false);
    setPromptForceSend(false);
    fetchPrompt.mutate({ name: currentPrompt.name, args: promptArgs });
  }

  return (
    <section className="flex h-full flex-col">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="text-xl font-semibold text-slate-800">接口</h2>
        <select
          className="rounded border border-slate-200 px-2 py-1 text-sm"
          value={currentServer?.id ?? ""}
          onChange={(e) => {
            setServerId(e.target.value);
            resetCallState();
          }}
        >
          {(servers.data ?? []).map((server) => (
            <option key={server.id} value={server.id}>
              {server.name}
              {server.state.state === "ready" ? "" : "（未连接）"}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-40"
          disabled={!ready}
          onClick={() => refresh.mutate()}
        >
          {refresh.isPending ? "刷新中…" : "刷新接口"}
        </button>
        {capabilities.data?.stale ? (
          <span className="text-xs text-amber-600">
            数据可能过期（上次刷新失败）
          </span>
        ) : null}
      </div>

      {capabilities.data?.warnings.length ? (
        <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700">
          <div className="font-medium">部分能力未启用或枚举失败：</div>
          <ul className="mt-1 list-disc space-y-0.5 pl-5">
            {capabilities.data.warnings.map((warning, index) => (
              <li key={`${index}:${warning}`}>{warning}</li>
            ))}
          </ul>
        </div>
      ) : null}

      {replayBanner ? (
        <div className="mb-3 flex items-center gap-2 rounded border border-sky-200 bg-sky-50 px-3 py-2 text-xs text-sky-700">
          <span className="truncate">{replayBanner}</span>
          <button
            type="button"
            className="ml-auto rounded border border-sky-200 px-2 py-0.5 text-[11px] hover:bg-sky-100"
            onClick={() => setReplayBanner(null)}
          >
            关闭
          </button>
        </div>
      ) : null}

      <div className="mb-3 flex items-center gap-2">
        {TABS.map((item) => (
          <button
            key={item.key}
            type="button"
            className={`rounded px-2 py-1 text-xs ${
              viewTab === item.key
                ? "bg-slate-800 text-white"
                : "border border-slate-200 text-slate-600 hover:bg-slate-50"
            }`}
            onClick={() => {
              setTab(item.key);
              resetCallState();
            }}
          >
            {item.label}
            <span className="ml-1 text-[10px] opacity-70">
              {item.key === "tools"
                ? (capabilities.data?.tools.length ?? 0)
                : item.key === "resources"
                  ? (capabilities.data?.resources.length ?? 0)
                  : item.key === "templates"
                    ? (capabilities.data?.resource_templates.length ?? 0)
                    : (capabilities.data?.prompts.length ?? 0)}
            </span>
          </button>
        ))}
        <input
          className="ml-auto w-56 rounded border border-slate-200 px-2 py-1 text-xs"
          placeholder="搜索接口…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>

      {servers.isLoading ? (
        <div className="text-sm text-slate-400">加载服务器中…</div>
      ) : servers.isError ? (
        <div className="text-sm text-red-500">
          服务器列表加载失败：{String(servers.error)}
        </div>
      ) : missingServer ? (
        <EmptyState
          title="服务器不存在"
          description="历史记录所属服务器可能已被删除，无法继续重放。"
        />
      ) : !currentServer ? (
        <EmptyState
          title="还没有服务器"
          description="请先在「服务器」页添加并连接一个服务器。"
        />
      ) : !ready ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <EmptyState
            title="服务器未连接"
            description="切换到「服务器」页连接后即可枚举与调用接口。"
          />
        </div>
      ) : capabilities.isLoading ? (
        <div className="text-sm text-slate-400">枚举接口中…</div>
      ) : capabilities.isError ? (
        <div className="text-sm text-red-500">
          枚举失败：{String(capabilities.error)}
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 gap-4">
          <div className="w-72 shrink-0 overflow-y-auto rounded-lg border border-slate-200 bg-white">
            {items.length === 0 ? (
              <div className="p-3 text-xs text-slate-400">该分类为空</div>
            ) : (
              items.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  ref={viewSelected === item.key ? selectedItemRef : undefined}
                  className={`block w-full border-b border-slate-100 px-3 py-2 text-left text-sm last:border-b-0 ${
                    viewSelected === item.key
                      ? "bg-slate-100"
                      : "hover:bg-slate-50"
                  }`}
                  onClick={() => {
                    resetCallState();
                    setSelected(item.key);
                  }}
                >
                  <div className="truncate text-slate-800">{item.title}</div>
                  {item.subtitle ? (
                    <div className="truncate text-xs text-slate-400">
                      {item.subtitle}
                    </div>
                  ) : null}
                </button>
              ))
            )}
          </div>

          <div className="min-w-0 flex-1 space-y-3 overflow-y-auto">
            {!viewSelected ? (
              <EmptyState
                title="选择一个接口"
                description="左侧列表选择后在此发起调用。"
              />
            ) : viewTab === "tools" && currentTool ? (
              <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4">
                <div>
                  <div className="flex items-center gap-2">
                    <div className="text-sm font-medium text-slate-800">
                      {currentTool.name}
                    </div>
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                      onClick={() =>
                        navigate(`/tests?target=${encodeURIComponent(currentTool.name)}`)
                      }
                    >
                      生成用例
                    </button>
                  </div>
                  {currentTool.description ? (
                    <div className="mt-1 text-xs text-slate-500">
                      {currentTool.description}
                    </div>
                  ) : null}
                </div>
                <details>
                  <summary className="cursor-pointer text-xs text-slate-400">
                    原始 JSON Schema
                  </summary>
                  <div className="mt-2">
                    <JsonViewer
                      value={currentTool.input_schema}
                      defaultMode="text"
                      contentClassName="max-h-48 overflow-auto"
                    />
                  </div>
                </details>
                <SchemaForm
                  key={`${currentTool.name}:${formNonce}`}
                  ref={toolFormRef}
                  schema={currentTool.input_schema}
                  initial={toolArgs}
                  onChange={handleToolArgsChange}
                />
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                    disabled={sendTool.isPending}
                    onClick={() => submitTool(false)}
                  >
                    {sendTool.isPending ? "调用中…" : "发送调用"}
                  </button>
                  {toolForceSend ? (
                    <button
                      type="button"
                      className="rounded border border-amber-300 bg-amber-50 px-3 py-1.5 text-sm text-amber-800 hover:bg-amber-100 disabled:opacity-50"
                      disabled={sendTool.isPending}
                      title="跳过前端校验直接发送，服务端可能返回参数错误"
                      onClick={() => submitTool(true)}
                    >
                      仍要发送
                    </button>
                  ) : null}
                  {inFlightId ? (
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-3 py-1.5 text-sm hover:bg-slate-50"
                      onClick={() => {
                        void cancelCall(currentServer.id, inFlightId);
                      }}
                    >
                      取消
                    </button>
                  ) : null}
                </div>
              </div>
            ) : viewTab === "resources" ? (
              <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4">
                <div className="break-all font-mono text-xs text-slate-600">
                  {viewSelected}
                </div>
                {!resourceKnown ? (
                  <div className="text-[11px] text-amber-600">
                    该资源不在当前枚举列表中，可直接按 URI 读取。
                  </div>
                ) : null}
                <button
                  type="button"
                  className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                  disabled={readRes.isPending}
                  onClick={() => readRes.mutate(viewSelected)}
                >
                  {readRes.isPending ? "读取中…" : "读取资源"}
                </button>
              </div>
            ) : viewTab === "templates" && currentTemplate ? (
              <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4">
                <div className="break-all font-mono text-xs text-slate-600">
                  {currentTemplate.uri_template}
                </div>
                {templateNames.map((name) => {
                  const missing = isBlank(viewTemplateVars[name]);
                  return (
                    <label
                      key={name}
                      className="block space-y-1 text-xs text-slate-600"
                    >
                      <span className="flex flex-wrap items-center gap-1">
                        <span className="text-red-500" aria-hidden="true">
                          *
                        </span>
                        {name}
                        <span className="rounded bg-red-50 px-1 py-0.5 text-[10px] font-medium text-red-600">
                          必填
                        </span>
                      </span>
                      <input
                        aria-label={name}
                        aria-required="true"
                        aria-invalid={missing || undefined}
                        className={`w-full rounded border px-2 py-1 text-sm ${
                          missing
                            ? "border-red-300 bg-red-50/40"
                            : "border-slate-200"
                        }`}
                        value={viewTemplateVars[name] ?? ""}
                        onChange={(e) =>
                          setTemplateVars((prev) => ({
                            ...prev,
                            [name]: e.target.value,
                          }))
                        }
                      />
                    </label>
                  );
                })}
                {missingTemplateVars.length > 0 ? (
                  <div className="text-[11px] text-red-500">
                    请填写全部模板变量：{missingTemplateVars.join("、")}
                  </div>
                ) : null}
                <button
                  type="button"
                  className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                  disabled={readRes.isPending || missingTemplateVars.length > 0}
                  onClick={() => {
                    const uri = templateNames.reduce(
                      (acc, name) =>
                        acc.replace(`{${name}}`, viewTemplateVars[name] ?? ""),
                      currentTemplate.uri_template,
                    );
                    readRes.mutate(uri);
                  }}
                >
                  {readRes.isPending ? "读取中…" : "生成 URI 并读取"}
                </button>
              </div>
            ) : viewTab === "prompts" && currentPrompt ? (
              <div className="space-y-3 rounded-lg border border-slate-200 bg-white p-4">
                <div className="text-sm font-medium text-slate-800">
                  {currentPrompt.name}
                </div>
                {currentPrompt.arguments.map((arg) => {
                  const missing =
                    arg.required && isBlank(String(promptArgs[arg.name] ?? ""));
                  const showMissing = promptSubmitted && missing;
                  return (
                    <label
                      key={arg.name}
                      className="block space-y-1 text-xs text-slate-600"
                    >
                      <span className="flex flex-wrap items-center gap-1">
                        {arg.required ? (
                          <span className="text-red-500" aria-hidden="true">
                            *
                          </span>
                        ) : null}
                        {arg.name}
                        {arg.required ? (
                          <span className="rounded bg-red-50 px-1 py-0.5 text-[10px] font-medium text-red-600">
                            必填
                          </span>
                        ) : null}
                      </span>
                      <input
                        aria-label={arg.name}
                        aria-required={arg.required || undefined}
                        aria-invalid={showMissing || undefined}
                        className={`w-full rounded border px-2 py-1 text-sm ${
                          showMissing
                            ? "border-red-300 bg-red-50/40"
                            : "border-slate-200"
                        }`}
                        value={String(promptArgs[arg.name] ?? "")}
                        onChange={(e) => {
                          setPromptArgs((prev) => ({
                            ...prev,
                            [arg.name]: e.target.value,
                          }));
                          setPromptForceSend(false);
                        }}
                      />
                    </label>
                  );
                })}
                {promptSubmitted && missingPromptArgs.length > 0 ? (
                  <div className="text-[11px] text-red-500">
                    缺少必填参数：{missingPromptArgs.join("、")}
                  </div>
                ) : null}
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                    disabled={fetchPrompt.isPending}
                    onClick={() => submitPrompt(false)}
                  >
                    {fetchPrompt.isPending ? "获取中…" : "获取提示词"}
                  </button>
                  {promptForceSend ? (
                    <button
                      type="button"
                      className="rounded border border-amber-300 bg-amber-50 px-3 py-1.5 text-sm text-amber-800 hover:bg-amber-100 disabled:opacity-50"
                      disabled={fetchPrompt.isPending}
                      title="跳过前端校验直接发送，服务端可能返回参数错误"
                      onClick={() => submitPrompt(true)}
                    >
                      仍要发送
                    </button>
                  ) : null}
                </div>
              </div>
            ) : (
              <div className="space-y-3 rounded-lg border border-amber-200 bg-amber-50 p-4">
                <div className="break-all text-sm font-medium text-amber-800">
                  未在当前接口列表中找到「{viewSelected}」
                </div>
                <div className="text-xs text-amber-700">
                  该接口可能已被服务端移除，或当前枚举结果已过期。参数仍可查看，可刷新接口后重试。
                </div>
                <button
                  type="button"
                  className="rounded border border-amber-300 px-2 py-1 text-xs text-amber-800 hover:bg-amber-100 disabled:opacity-40"
                  disabled={!ready || refresh.isPending}
                  onClick={() => refresh.mutate()}
                >
                  {refresh.isPending ? "刷新中…" : "刷新接口"}
                </button>
                {viewTab === "templates" ? (
                  <div className="break-all font-mono text-xs text-slate-600">
                    URI：{viewSelected}
                  </div>
                ) : (
                  <JsonViewer
                    value={viewTab === "tools" ? toolArgs : promptArgs}
                    defaultMode="tree"
                    contentClassName="max-h-40 overflow-auto"
                  />
                )}
              </div>
            )}

            {result ? <ResultViewer record={result} /> : null}
          </div>
        </div>
      )}
      {currentServer ? (
        <EventPanel key={currentServer.id} serverId={currentServer.id} />
      ) : null}
    </section>
  );
}
