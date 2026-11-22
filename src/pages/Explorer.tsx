import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { EventPanel } from "../components/EventPanel";
import { ExportPreviewDialog } from "../components/ExportPreviewDialog";
import { JsonViewer } from "../components/JsonViewer";
import {
  RecentParamsButton,
  type RecentParamsContext,
} from "../components/RecentParamsButton";
import { ResultViewer } from "../components/ResultViewer";
import {
  SchemaForm,
  type SchemaFormHandle,
  type SchemaFormSnapshot,
} from "../components/SchemaForm";
import { ServerSelect } from "../components/ServerSelect";
import { ServerStatusActions } from "../components/ServerStatusActions";
import {
  getCapabilities,
  listServers,
  refreshCapabilities,
  exportCapabilityExport,
  previewCapabilityExport,
  type CapabilityExportFormat,
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
import { supportsHeaderParams, toolHeaderParams } from "../lib/headerParams";
import {
  buildInterfaceEntries,
  searchInterfaceEntries,
  type InterfaceSearchHit,
  type SearchScope,
} from "../lib/interfaceSearch";
import { filterKnownTopLevel } from "../lib/schema";
import { pickDefaultServerId } from "../lib/serverOrder";
import { draftKey, useDraftStore } from "../stores/drafts";
import { useReplayStore } from "../stores/replay";
import { toast } from "../stores/toast";
import { formatDateTime } from "../components/json/format";
import type {
  CallRecord,
  CapabilitySnapshot,
  RecentCallParam,
} from "../types/models";

type Tab = "tools" | "resources" | "templates" | "prompts";

type ResultContext = {
  serverId: string;
  method: "tools_call" | "resources_read" | "prompts_get";
  view: Tab;
  target: string;
  resolvedTarget?: string;
};

type HistoryNotice =
  | {
      kind: "tool";
      context: RecentParamsContext;
      label: string;
      previousSnapshot: SchemaFormSnapshot | null;
      dropped: number;
      unknown: number;
      disallowsAdditional: boolean;
      empty: boolean;
    }
  | {
      kind: "prompt";
      context: RecentParamsContext;
      label: string;
      previousArgs: Record<string, unknown>;
      dropped: number;
      unknown: number;
      disallowsAdditional: boolean;
      empty: boolean;
    };

const TABS: { key: Tab; label: string }[] = [
  { key: "tools", label: "Tools" },
  { key: "resources", label: "Resources" },
  { key: "templates", label: "资源模板" },
  { key: "prompts", label: "Prompts" },
];

/** 在文本中高亮所有大小写不敏感的关键词命中 */
function HighlightText({ text, keyword }: { text: string; keyword: string }) {
  const needle = keyword.trim();
  if (!needle) return <>{text}</>;

  const lowerText = text.toLowerCase();
  const lowerNeedle = needle.toLowerCase();
  const parts: ReactNode[] = [];
  let cursor = 0;
  let index = lowerText.indexOf(lowerNeedle);

  while (index >= 0) {
    if (index > cursor) parts.push(text.slice(cursor, index));
    parts.push(
      <mark key={index} className="rounded-sm bg-amber-100 px-0.5 text-inherit">
        {text.slice(index, index + needle.length)}
      </mark>,
    );
    cursor = index + needle.length;
    index = lowerText.indexOf(lowerNeedle, Math.max(cursor, index + 1));
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return <>{parts}</>;
}

function SearchGroupHeader({ label }: { label: string }) {
  return (
    <div className="sticky top-0 z-10 border-b border-slate-100 bg-slate-50 px-3 py-1 text-[11px] font-medium text-slate-500">
      {label}
    </div>
  );
}

/** 参数是否为空（用于模板变量必填提示） */
function isBlank(value: string | undefined): boolean {
  return !(value ?? "").trim();
}

function viewLabel(view: Tab): string {
  switch (view) {
    case "tools":
      return "工具";
    case "prompts":
      return "提示词";
    case "templates":
      return "资源模板";
    default:
      return "资源";
  }
}

/** 导出文件名 slug：过滤 Windows 非法字符，截断 40 字符 */
function exportSlug(name: string): string {
  const slug = name
    .replace(/[\\/:*?"<>|\s]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
  return slug || "server";
}

function exportTimestamp(): string {
  return new Date()
    .toISOString()
    .replace(/[-:]/g, "")
    .replace("T", "-")
    .slice(0, 15);
}

/** 接口页：枚举 + 调用台（Tools / Resources / 模板 / Prompts） */
export function Explorer() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const servers = useQuery({ queryKey: ["servers"], queryFn: listServers });
  const appInfo = useQuery({ queryKey: ["appInfo"], queryFn: getAppInfo });
  const serverList = servers.data ?? [];
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
  const [searchScope, setSearchScope] = useState<SearchScope>("name");
  const [selected, setSelected] = useState<string>(() => replay?.target ?? "");
  const [result, setResult] = useState<CallRecord | null>(null);
  const [resultContext, setResultContext] = useState<ResultContext | null>(
    null,
  );
  const [resultCollapsed, setResultCollapsed] = useState(false);
  const [eventPanelServerId, setEventPanelServerId] = useState<string | null>(
    null,
  );
  const [inFlightId, setInFlightId] = useState<string | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportFormat, setExportFormat] =
    useState<CapabilityExportFormat>("csv");
  const [includeSchema, setIncludeSchema] = useState(false);
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
  const [historyNotice, setHistoryNotice] = useState<HistoryNotice | null>(
    null,
  );
  const [pendingResource, setPendingResource] = useState<string | null>(() =>
    replay?.method === "resources_read" ? replay.target : null,
  );
  const selectedItemRef = useRef<HTMLButtonElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const toolFormRef = useRef<SchemaFormHandle | null>(null);

  const currentServer = serverId
    ? serverList.find((s) => s.id === serverId)
    : serverList.find(
        (s) =>
          s.id ===
          pickDefaultServerId(serverList, appInfo.data?.default_server_id),
      );
  const currentServerId = currentServer?.id;
  const eventPanelOpen =
    Boolean(currentServerId) && eventPanelServerId === currentServerId;
  const missingServer =
    Boolean(serverId) && servers.isSuccess && !currentServer;
  const ready = currentServer?.state.state === "ready";
  const readyHandshake =
    currentServer?.state.state === "ready"
      ? currentServer.state.handshake
      : null;
  const headerParamsEnabled = Boolean(
    currentServer &&
    supportsHeaderParams(
      currentServer.transport,
      readyHandshake?.protocol_version ?? null,
    ),
  );

  const capabilities = useQuery<CapabilitySnapshot>({
    queryKey: ["capabilities", currentServer?.id],
    queryFn: () => getCapabilities(currentServer?.id ?? ""),
    enabled: Boolean(currentServer?.id) && ready,
  });

  // 重放值已在首帧由 store 初始化；这里清空一次性标记，并把工具/提示词重放写入草稿。
  useEffect(() => {
    if (!replay) return;
    if (replay.method === "tools_call" || replay.method === "prompts_get") {
      useDraftStore
        .getState()
        .save(
          draftKey(replay.serverId, replay.method, replay.target),
          replayArguments(replay.params),
        );
    }
    setReplay(null);
  }, [replay, setReplay]);

  // 启动配置指定的默认服务器由 currentServer 派生，不写入本地 state，
  // 避免 effect 内同步 setState；用户选择后 serverId 优先生效。

  // resources/read 可能是模板生成的 URI：能力快照到位后再派生具体资源/模板视图
  const resourceReplay = (() => {
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
  })();

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

  // Ctrl/Cmd+F 聚焦接口搜索框
  useEffect(() => {
    function handleFindShortcut(event: globalThis.KeyboardEvent) {
      if (
        !(event.ctrlKey || event.metaKey) ||
        event.shiftKey ||
        event.altKey ||
        event.key.toLowerCase() !== "f"
      ) {
        return;
      }
      event.preventDefault();
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    }

    window.addEventListener("keydown", handleFindShortcut);
    return () => window.removeEventListener("keydown", handleFindShortcut);
  }, []);

  function acceptResult(record: CallRecord, context: ResultContext) {
    setResult(record);
    setResultContext(context);
    setResultCollapsed(eventPanelOpen);
    void queryClient.invalidateQueries({ queryKey: ["recent-params"] });
  }

  const refresh = useMutation({
    mutationFn: () => refreshCapabilities(currentServer?.id ?? ""),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(["capabilities", currentServer?.id], snapshot);
    },
    onError: (error) => toast.fromError(error, "刷新接口失败"),
  });

  const sendTool = useMutation({
    mutationFn: async (payload: {
      name: string;
      args: Record<string, unknown>;
      context: ResultContext;
    }) => {
      const callId = crypto.randomUUID();
      setInFlightId(callId);
      try {
        return await callTool(
          payload.context.serverId,
          callId,
          payload.name,
          payload.args,
        );
      } finally {
        setInFlightId(null);
      }
    },
    onSuccess: (record, payload) => acceptResult(record, payload.context),
    onError: (error) => toast.fromError(error, "调用失败"),
  });

  const readRes = useMutation({
    mutationFn: (payload: { uri: string; context: ResultContext }) =>
      readResource(payload.context.serverId, payload.uri),
    onSuccess: (record, payload) => acceptResult(record, payload.context),
    onError: (error) => toast.fromError(error, "读取资源失败"),
  });

  const fetchPrompt = useMutation({
    mutationFn: (payload: {
      name: string;
      args: Record<string, unknown>;
      context: ResultContext;
    }) => getPrompt(payload.context.serverId, payload.name, payload.args),
    onSuccess: (record, payload) => acceptResult(record, payload.context),
    onError: (error) => toast.fromError(error, "获取提示词失败"),
  });

  const interfaceEntries = useMemo(
    () =>
      capabilities.data
        ? buildInterfaceEntries(capabilities.data, viewTab)
        : [],
    [capabilities.data, viewTab],
  );
  const search = useMemo(
    () => searchInterfaceEntries(interfaceEntries, filter, searchScope),
    [interfaceEntries, filter, searchScope],
  );
  const hasKeyword = Boolean(filter.trim());
  const selectedFilteredOut = Boolean(
    viewSelected &&
    hasKeyword &&
    interfaceEntries.some((entry) => entry.key === viewSelected) &&
    ![...search.nameHits, ...search.contentHits].some(
      (entry) => entry.key === viewSelected,
    ),
  );

  const currentTool = (capabilities.data?.tools ?? []).find(
    (t) => t.name === viewSelected,
  );
  const toolHeaderParamList = useMemo(
    () =>
      headerParamsEnabled && currentTool
        ? toolHeaderParams(currentTool.input_schema)
        : [],
    [headerParamsEnabled, currentTool],
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
  const resolvedTemplateUri = currentTemplate
    ? templateNames.reduce(
        (uri, name) => uri.replace(`{${name}}`, viewTemplateVars[name] ?? ""),
        currentTemplate.uri_template,
      )
    : "";
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

  const resultMatches = Boolean(
    result &&
    resultContext &&
    currentServerId &&
    resultContext.serverId === currentServerId &&
    resultContext.view === viewTab &&
    ((viewTab === "tools" &&
      resultContext.method === "tools_call" &&
      currentTool &&
      resultContext.target === currentTool.name) ||
      (viewTab === "prompts" &&
        resultContext.method === "prompts_get" &&
        currentPrompt &&
        resultContext.target === currentPrompt.name) ||
      (viewTab === "resources" &&
        resultContext.method === "resources_read" &&
        resultContext.target === viewSelected) ||
      (viewTab === "templates" &&
        resultContext.method === "resources_read" &&
        currentTemplate &&
        resultContext.target === currentTemplate.uri_template)),
  );

  function clearSelectionState() {
    setSelected("");
    setToolArgs({});
    setPromptArgs({});
    setTemplateVars({});
    setPendingResource(null);
    setToolForceSend(false);
    setPromptSubmitted(false);
    setPromptForceSend(false);
    setHistoryNotice(null);
    setFormNonce((value) => value + 1);
  }

  function clearResultState() {
    setResult(null);
    setResultContext(null);
    setResultCollapsed(false);
  }

  function resetCallState() {
    clearSelectionState();
    clearResultState();
    setEventPanelServerId(null);
  }

  function updatePromptArgs(next: Record<string, unknown>) {
    setPromptArgs(next);
    setPromptForceSend(false);
    setHistoryNotice(null);
    if (currentServer && currentPrompt) {
      useDraftStore
        .getState()
        .save(
          draftKey(currentServer.id, "prompts_get", currentPrompt.name),
          next,
        );
    }
  }

  function updateTemplateVars(next: Record<string, string>) {
    setTemplateVars(next);
    if (currentServer && currentTemplate) {
      useDraftStore
        .getState()
        .save(
          draftKey(
            currentServer.id,
            "resources_read",
            currentTemplate.uri_template,
          ),
          next,
        );
    }
  }

  function handleToolArgsChange(next: Record<string, unknown>) {
    setToolArgs(next);
    setToolForceSend(false);
    setHistoryNotice(null);
    if (currentServer && currentTool) {
      useDraftStore
        .getState()
        .save(draftKey(currentServer.id, "tools_call", currentTool.name), next);
    }
  }

  function selectItem(key: string) {
    setTab(viewTab);
    clearSelectionState();
    setSelected(key);
    if (!currentServer) return;

    if (viewTab === "tools") {
      const nextTool = capabilities.data?.tools.find(
        (tool) => tool.name === key,
      );
      const draft = useDraftStore
        .getState()
        .get(draftKey(currentServer.id, "tools_call", key));
      setToolArgs(
        draft ? filterKnownTopLevel(nextTool?.input_schema, draft).value : {},
      );
      return;
    }

    if (viewTab === "prompts") {
      const nextPrompt = capabilities.data?.prompts.find(
        (prompt) => prompt.name === key,
      );
      const names = new Set(
        (nextPrompt?.arguments ?? []).map((arg) => arg.name),
      );
      const draft = useDraftStore
        .getState()
        .get(draftKey(currentServer.id, "prompts_get", key));
      setPromptArgs(
        draft
          ? Object.fromEntries(
              Object.entries(draft)
                .filter(([name]) => names.has(name))
                .map(([name, value]) => [name, String(value ?? "")]),
            )
          : {},
      );
      return;
    }

    if (viewTab === "templates") {
      const draft = useDraftStore
        .getState()
        .get(draftKey(currentServer.id, "resources_read", key));
      setTemplateVars(
        draft
          ? Object.fromEntries(
              Object.entries(draft).map(([name, value]) => [
                name,
                String(value ?? ""),
              ]),
            )
          : {},
      );
    }
  }

  function submitTool(force: boolean) {
    if (!currentTool || !currentServer || sendTool.isPending) return;
    const form = toolFormRef.current;
    if (!form) return;

    const context: ResultContext = {
      serverId: currentServer.id,
      method: "tools_call",
      view: "tools",
      target: currentTool.name,
    };

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
      sendTool.mutate({ name: currentTool.name, args: result.value, context });
      return;
    }

    const result = form.readValue();
    // JSON 解析失败时无法确定要发送什么，退回到展示错误
    if (result.parseError) {
      form.validate();
      return;
    }
    setToolForceSend(false);
    sendTool.mutate({ name: currentTool.name, args: result.value, context });
  }

  function submitPrompt(force: boolean) {
    if (!currentPrompt || !currentServer || fetchPrompt.isPending) return;
    if (!force && missingPromptArgs.length > 0) {
      setPromptSubmitted(true);
      setPromptForceSend(true);
      return;
    }
    setPromptSubmitted(false);
    setPromptForceSend(false);
    fetchPrompt.mutate({
      name: currentPrompt.name,
      args: promptArgs,
      context: {
        serverId: currentServer.id,
        method: "prompts_get",
        view: "prompts",
        target: currentPrompt.name,
      },
    });
  }

  function handleShortcut(event: KeyboardEvent, action: () => void) {
    if (event.repeat) return;
    if (!(event.ctrlKey || event.metaKey) || event.key !== "Enter") return;
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) {
      return;
    }
    event.preventDefault();
    action();
  }

  function readTemplate() {
    if (!currentTemplate || !currentServer || readRes.isPending) return;
    if (missingTemplateVars.length > 0) return;
    readRes.mutate({
      uri: resolvedTemplateUri,
      context: {
        serverId: currentServer.id,
        method: "resources_read",
        view: "templates",
        target: currentTemplate.uri_template,
        resolvedTarget: resolvedTemplateUri,
      },
    });
  }

  function historyContextMatches(context: RecentParamsContext): boolean {
    if (!currentServer || context.serverId !== currentServer.id) return false;
    return (
      (context.method === "tools_call" &&
        viewTab === "tools" &&
        currentTool?.name === context.target) ||
      (context.method === "prompts_get" &&
        viewTab === "prompts" &&
        currentPrompt?.name === context.target)
    );
  }

  function applyToolHistory(
    params: Record<string, unknown>,
    item: RecentCallParam,
    context: RecentParamsContext,
  ) {
    if (!currentServer || !currentTool) return;
    if (!historyContextMatches(context)) return;
    const filtered = filterKnownTopLevel(currentTool.input_schema, params);
    const previousSnapshot = toolFormRef.current?.snapshot() ?? null;
    setToolArgs(filtered.value);
    toolFormRef.current?.apply(filtered.value);
    setHistoryNotice({
      kind: "tool",
      context,
      label: formatDateTime(item.created_at),
      previousSnapshot,
      dropped: filtered.dropped,
      unknown: filtered.unknown,
      disallowsAdditional: filtered.disallowsAdditional,
      empty: Object.keys(params).length === 0,
    });
  }

  function applyPromptHistory(
    params: Record<string, unknown>,
    item: RecentCallParam,
    context: RecentParamsContext,
  ) {
    if (!currentServer || !currentPrompt) return;
    if (!historyContextMatches(context)) return;
    const names = new Set(currentPrompt.arguments.map((arg) => arg.name));
    const value: Record<string, string> = {};
    let dropped = 0;
    for (const [key, raw] of Object.entries(params)) {
      if (names.has(key)) value[key] = String(raw ?? "");
      else dropped += 1;
    }
    const previousArgs = promptArgs;
    updatePromptArgs(value);
    setHistoryNotice({
      kind: "prompt",
      context,
      label: formatDateTime(item.created_at),
      previousArgs,
      dropped,
      unknown: 0,
      disallowsAdditional: false,
      empty: Object.keys(params).length === 0,
    });
  }

  function undoHistory() {
    const notice = historyNotice;
    if (!notice || !historyContextMatches(notice.context)) return;
    if (notice.kind === "tool") {
      if (!notice.previousSnapshot) return;
      toolFormRef.current?.restore(notice.previousSnapshot);
    } else {
      updatePromptArgs(notice.previousArgs);
    }
    setHistoryNotice(null);
  }

  function historyNoticeMatches(): boolean {
    return Boolean(
      historyNotice && historyContextMatches(historyNotice.context),
    );
  }

  function historyNoticeDetail(notice: HistoryNotice): string | null {
    if (notice.empty) return "该历史记录没有参数";
    if (notice.dropped > 0) {
      return `已忽略 ${notice.dropped} 个当前 Schema 不允许的字段`;
    }
    if (notice.disallowsAdditional && notice.unknown > 0) {
      return `当前 Schema 不允许额外字段，已保留 ${notice.unknown} 个，发送可能被拒`;
    }
    if (notice.unknown > 0) {
      return `有 ${notice.unknown} 个字段未在当前 Schema 声明，已按历史原样保留；可切换 JSON 模式查看`;
    }
    return null;
  }

  function renderSearchItem(item: InterfaceSearchHit) {
    const isContent = item.matchedIn === "content";
    return (
      <button
        key={item.key}
        type="button"
        ref={viewSelected === item.key ? selectedItemRef : undefined}
        className={`block w-full border-b border-slate-100 px-3 py-2 text-left text-sm last:border-b-0 ${
          viewSelected === item.key ? "bg-slate-100" : "hover:bg-slate-50"
        }`}
        onClick={() => selectItem(item.key)}
      >
        <div className="truncate text-slate-800">
          <HighlightText text={item.title} keyword={filter} />
        </div>
        {isContent && item.snippet ? (
          <div className="mt-0.5 flex items-start gap-1 text-xs text-slate-400">
            <span className="mt-px shrink-0 rounded bg-slate-100 px-1 text-[10px] leading-4 text-slate-500">
              {item.contentLabel ?? "内容"}
            </span>
            <span className="line-clamp-2 min-w-0 break-words">
              <HighlightText text={item.snippet} keyword={filter} />
            </span>
          </div>
        ) : item.subtitle ? (
          <div className="truncate text-xs text-slate-400">
            <HighlightText text={item.subtitle} keyword={filter} />
          </div>
        ) : null}
      </button>
    );
  }

  return (
    <section className="flex h-full flex-col">
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <h2 className="text-xl font-semibold text-slate-800">接口</h2>
        <ServerSelect
          servers={serverList}
          value={currentServer?.id ?? ""}
          onChange={(id) => {
            setServerId(id);
            resetCallState();
          }}
          className="rounded border border-slate-200 px-2 py-1 text-sm"
          ariaLabel="选择服务器"
        />
        <ServerStatusActions key={currentServer?.id} server={currentServer} />
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-40"
          disabled={!ready}
          onClick={() => refresh.mutate()}
        >
          {refresh.isPending ? "刷新中…" : "刷新接口"}
        </button>
        <button
          type="button"
          className="rounded border border-slate-200 px-2 py-1 text-xs hover:bg-slate-50 disabled:opacity-40"
          disabled={!ready || !capabilities.data || !currentServerId}
          onClick={() => setExportOpen(true)}
        >
          导出接口
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
              if (item.key === viewTab) return;
              setTab(item.key);
              clearSelectionState();
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
            title={
              currentServer.state.state === "connecting"
                ? "服务器连接中"
                : currentServer.state.state === "degraded"
                  ? "服务器连接降级"
                  : currentServer.state.state === "lost" ||
                      currentServer.state.state === "error"
                    ? "服务器连接不可用"
                    : "服务器未连接"
            }
            description={
              currentServer.state.state === "connecting"
                ? "正在连接服务器，请稍候…"
                : currentServer.state.state === "degraded"
                  ? "连接已降级，请点击顶部「重连」恢复后继续。"
                  : currentServer.state.state === "lost"
                    ? "连接已掉线，请点击顶部「重连」恢复后继续。"
                    : currentServer.state.state === "error"
                      ? "连接出错，请点击顶部「重连」或前往「服务器」页查看详情。"
                      : "请点击顶部「连接」，或前往「服务器」页连接后继续。"
            }
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
          <div className="flex w-72 shrink-0 flex-col overflow-hidden rounded-lg border border-slate-200 bg-white">
            <div className="space-y-2 border-b border-slate-100 p-2">
              <div className="relative">
                <input
                  ref={searchInputRef}
                  aria-label="搜索接口"
                  className="w-full rounded border border-slate-200 px-2 py-1 pr-7 text-xs"
                  placeholder={
                    searchScope === "name"
                      ? "搜索接口名称或 URI…"
                      : "搜索名称、描述、参数…"
                  }
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key !== "Escape" || !filter) return;
                    event.preventDefault();
                    setFilter("");
                  }}
                />
                {filter ? (
                  <button
                    type="button"
                    aria-label="清除搜索"
                    className="absolute right-1 top-1/2 -translate-y-1/2 rounded px-1 text-sm leading-none text-slate-400 hover:text-slate-600"
                    onClick={() => {
                      setFilter("");
                      searchInputRef.current?.focus();
                    }}
                  >
                    ×
                  </button>
                ) : null}
              </div>
              <div className="flex items-center justify-between gap-2">
                <div className="flex overflow-hidden rounded border border-slate-200 text-[11px]">
                  <button
                    type="button"
                    aria-pressed={searchScope === "name"}
                    className={`px-2 py-0.5 ${
                      searchScope === "name"
                        ? "bg-slate-800 text-white"
                        : "text-slate-600 hover:bg-slate-50"
                    }`}
                    onClick={() => setSearchScope("name")}
                  >
                    名称
                  </button>
                  <button
                    type="button"
                    aria-pressed={searchScope === "all"}
                    title="同时搜索描述、参数和字段名"
                    className={`border-l border-slate-200 px-2 py-0.5 ${
                      searchScope === "all"
                        ? "bg-slate-800 text-white"
                        : "text-slate-600 hover:bg-slate-50"
                    }`}
                    onClick={() => setSearchScope("all")}
                  >
                    名称+内容
                  </button>
                </div>
                <span
                  aria-live="polite"
                  className="text-[11px] tabular-nums text-slate-400"
                >
                  {hasKeyword
                    ? `${search.total} / ${interfaceEntries.length}`
                    : `${interfaceEntries.length} 个`}
                </span>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {interfaceEntries.length === 0 ? (
                <div className="p-3 text-xs text-slate-400">该分类为空</div>
              ) : search.total === 0 ? (
                <div className="space-y-2 p-3 text-xs text-slate-400">
                  <div>没有匹配的接口</div>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="rounded border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                      onClick={() => setFilter("")}
                    >
                      清空搜索
                    </button>
                    {searchScope === "name" ? (
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                        onClick={() => setSearchScope("all")}
                      >
                        搜索全部内容
                      </button>
                    ) : null}
                  </div>
                </div>
              ) : (
                <>
                  {hasKeyword &&
                  search.nameHits.length > 0 &&
                  search.contentHits.length > 0 ? (
                    <SearchGroupHeader label="名称匹配" />
                  ) : null}
                  {search.nameHits.map(renderSearchItem)}
                  {search.contentHits.length > 0 ? (
                    <SearchGroupHeader label="内容匹配" />
                  ) : null}
                  {search.contentHits.map(renderSearchItem)}
                </>
              )}
              {selectedFilteredOut ? (
                <div className="sticky bottom-0 border-t border-amber-200 bg-amber-50 px-3 py-2 text-[11px] text-amber-700">
                  当前选中项不在筛选结果中
                </div>
              ) : null}
            </div>
          </div>

          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div
              className={
                resultMatches
                  ? "min-h-0 flex-[3] space-y-3 overflow-y-auto pr-1"
                  : "min-h-0 flex-1 space-y-3 overflow-y-auto pr-1"
              }
            >
              {!viewSelected ? (
                <EmptyState
                  title="选择一个接口"
                  description="左侧列表选择后在此发起调用。"
                />
              ) : viewTab === "tools" && currentTool ? (
                <div
                  className="space-y-3 rounded-lg border border-slate-200 bg-white p-4"
                  onKeyDown={(event) =>
                    handleShortcut(event, () => submitTool(false))
                  }
                >
                  <div>
                    <div className="flex items-center gap-2">
                      <div className="text-sm font-medium text-slate-800">
                        {currentTool.name}
                      </div>
                      <button
                        type="button"
                        className="rounded border border-slate-200 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-50"
                        onClick={() =>
                          navigate(
                            `/tests?target=${encodeURIComponent(currentTool.name)}`,
                          )
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
                    {toolHeaderParamList.length > 0 ? (
                      <div className="mt-1 text-[11px] text-sky-700">
                        该工具会将 {toolHeaderParamList.length} 个参数作为
                        Mcp-Param-* 请求头发送：
                        {toolHeaderParamList
                          .map((item) => `${item.argument} -> ${item.header}`)
                          .join("；")}
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
                    onDirty={() => setHistoryNotice(null)}
                    showHeaderParams={headerParamsEnabled}
                  />
                  {historyNoticeMatches() && historyNotice ? (
                    <div className="flex flex-wrap items-center gap-2 rounded border border-sky-200 bg-sky-50 px-3 py-2 text-xs text-sky-700">
                      <span>
                        已载入 {historyNotice.label} 的参数
                        {historyNoticeDetail(historyNotice)
                          ? `（${historyNoticeDetail(historyNotice)}）`
                          : ""}
                      </span>
                      <button
                        type="button"
                        className="rounded border border-sky-200 bg-white px-2 py-0.5 text-[11px] hover:bg-sky-100 disabled:opacity-40"
                        disabled={
                          historyNotice.kind === "tool" &&
                          !historyNotice.previousSnapshot
                        }
                        onClick={undoHistory}
                      >
                        撤销
                      </button>
                    </div>
                  ) : null}
                  <div className="sticky bottom-0 z-10 -mx-4 flex flex-wrap items-center gap-2 border-t border-slate-100 bg-white/95 px-4 py-2 backdrop-blur">
                    <button
                      type="button"
                      className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                      disabled={sendTool.isPending}
                      aria-keyshortcuts="Control+Enter Meta+Enter"
                      title="发送调用（Ctrl+Enter）"
                      onClick={() => submitTool(false)}
                    >
                      {sendTool.isPending ? "调用中…" : "发送调用"}
                    </button>
                    <RecentParamsButton
                      key={`${currentServer.id}:tools_call:${currentTool.name}`}
                      context={{
                        serverId: currentServer.id,
                        method: "tools_call",
                        target: currentTool.name,
                      }}
                      onApply={applyToolHistory}
                    />
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
                          void cancelCall(currentServer.id, inFlightId).catch(
                            (error) => toast.fromError(error, "取消失败"),
                          );
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
                    onClick={() =>
                      readRes.mutate({
                        uri: viewSelected,
                        context: {
                          serverId: currentServer.id,
                          method: "resources_read",
                          view: "resources",
                          target: viewSelected,
                        },
                      })
                    }
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
                            updateTemplateVars({
                              ...viewTemplateVars,
                              [name]: e.target.value,
                            })
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
                    disabled={
                      readRes.isPending || missingTemplateVars.length > 0
                    }
                    onClick={readTemplate}
                  >
                    {readRes.isPending ? "读取中…" : "生成 URI 并读取"}
                  </button>
                  {resolvedTemplateUri ? (
                    <div className="break-all text-[11px] text-slate-400">
                      即将读取：{resolvedTemplateUri}
                    </div>
                  ) : null}
                </div>
              ) : viewTab === "prompts" && currentPrompt ? (
                <div
                  className="space-y-3 rounded-lg border border-slate-200 bg-white p-4"
                  onKeyDown={(event) =>
                    handleShortcut(event, () => submitPrompt(false))
                  }
                >
                  <div className="text-sm font-medium text-slate-800">
                    {currentPrompt.name}
                  </div>
                  {currentPrompt.arguments.map((arg) => {
                    const missing =
                      arg.required &&
                      isBlank(String(promptArgs[arg.name] ?? ""));
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
                          onChange={(e) =>
                            updatePromptArgs({
                              ...promptArgs,
                              [arg.name]: e.target.value,
                            })
                          }
                        />
                      </label>
                    );
                  })}
                  {promptSubmitted && missingPromptArgs.length > 0 ? (
                    <div className="text-[11px] text-red-500">
                      缺少必填参数：{missingPromptArgs.join("、")}
                    </div>
                  ) : null}
                  {historyNoticeMatches() && historyNotice ? (
                    <div className="flex flex-wrap items-center gap-2 rounded border border-sky-200 bg-sky-50 px-3 py-2 text-xs text-sky-700">
                      <span>
                        已载入 {historyNotice.label} 的参数
                        {historyNoticeDetail(historyNotice)
                          ? `（${historyNoticeDetail(historyNotice)}）`
                          : ""}
                      </span>
                      <button
                        type="button"
                        className="rounded border border-sky-200 bg-white px-2 py-0.5 text-[11px] hover:bg-sky-100"
                        onClick={undoHistory}
                      >
                        撤销
                      </button>
                    </div>
                  ) : null}
                  <div className="sticky bottom-0 z-10 -mx-4 flex flex-wrap items-center gap-2 border-t border-slate-100 bg-white/95 px-4 py-2 backdrop-blur">
                    <button
                      type="button"
                      className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
                      disabled={fetchPrompt.isPending}
                      aria-keyshortcuts="Control+Enter Meta+Enter"
                      title="获取提示词（Ctrl+Enter）"
                      onClick={() => submitPrompt(false)}
                    >
                      {fetchPrompt.isPending ? "获取中…" : "获取提示词"}
                    </button>
                    <RecentParamsButton
                      key={`${currentServer.id}:prompts_get:${currentPrompt.name}`}
                      context={{
                        serverId: currentServer.id,
                        method: "prompts_get",
                        target: currentPrompt.name,
                      }}
                      onApply={applyPromptHistory}
                    />
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
            </div>

            {result && !resultMatches ? (
              <div className="shrink-0 flex flex-wrap items-center gap-2 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700">
                <span>
                  上一次调用结果属于 {viewLabel(resultContext?.view ?? "tools")}
                  「
                  {resultContext?.resolvedTarget ??
                    resultContext?.target ??
                    result.target}
                  」，已切换接口，未在此处展示。
                </span>
                <button
                  type="button"
                  className="rounded border border-amber-300 bg-white px-2 py-0.5 text-[11px] hover:bg-amber-100"
                  onClick={clearResultState}
                >
                  清除
                </button>
              </div>
            ) : null}

            {result && resultMatches ? (
              <div
                className={
                  resultCollapsed || eventPanelOpen
                    ? "shrink-0 overflow-hidden rounded-lg border border-slate-200 bg-white"
                    : "flex min-h-[140px] flex-[2] flex-col overflow-hidden rounded-lg border border-slate-200 bg-white"
                }
              >
                <button
                  type="button"
                  className="flex shrink-0 items-center gap-2 border-b border-slate-100 px-3 py-1.5 text-left text-xs text-slate-600 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-70"
                  aria-expanded={!resultCollapsed && !eventPanelOpen}
                  disabled={eventPanelOpen}
                  title={
                    eventPanelOpen
                      ? "关闭事件面板后查看结果"
                      : resultCollapsed
                        ? "展开结果"
                        : "收起结果"
                  }
                  onClick={() => setResultCollapsed((current) => !current)}
                >
                  <span>
                    结果 ·{" "}
                    {result.status === "ok"
                      ? "成功"
                      : result.status === "error"
                        ? "失败"
                        : "取消"}{" "}
                    · {result.duration_ms}ms
                  </span>
                  <span className="ml-auto text-slate-400">
                    {eventPanelOpen
                      ? "关闭事件面板后查看"
                      : resultCollapsed
                        ? "展开"
                        : "收起"}
                  </span>
                </button>
                {resultCollapsed || eventPanelOpen ? null : (
                  <div className="min-h-0 flex-1" aria-live="polite">
                    <ResultViewer fill record={result} />
                  </div>
                )}
              </div>
            ) : null}
          </div>
        </div>
      )}
      {currentServer ? (
        <EventPanel
          key={currentServer.id}
          serverId={currentServer.id}
          onOpenChange={(open) => {
            setEventPanelServerId(open ? currentServer.id : null);
            if (open) setResultCollapsed(true);
          }}
        />
      ) : null}
      {exportOpen && currentServerId ? (
        <ExportPreviewDialog
          key={`${currentServerId}:${exportFormat}:${includeSchema}`}
          title="导出接口清单"
          defaultFilename={`${
            exportFormat === "csv"
              ? "mcptester-interfaces"
              : exportFormat === "csv_params"
                ? "mcptester-interface-params"
                : "mcptester-capabilities"
          }-${exportSlug(currentServer?.name ?? "server")}-${exportTimestamp()}.${
            exportFormat === "json" ? "json" : "csv"
          }`}
          load={() =>
            previewCapabilityExport(
              currentServerId,
              exportFormat,
              includeSchema,
            )
          }
          onSave={(path) => {
            if (!currentServerId) {
              return Promise.reject(new Error("服务器未连接"));
            }
            return exportCapabilityExport(
              currentServerId,
              exportFormat,
              path,
              includeSchema,
            );
          }}
          onClose={() => setExportOpen(false)}
          filters={
            exportFormat === "json"
              ? [{ name: "JSON", extensions: ["json"] }]
              : [{ name: "CSV", extensions: ["csv"] }]
          }
          extra={
            <div className="flex items-center gap-2 text-xs text-slate-600">
              <select
                aria-label="导出格式"
                className="rounded border border-slate-200 px-2 py-1 text-xs"
                value={exportFormat}
                onChange={(event) =>
                  setExportFormat(event.target.value as CapabilityExportFormat)
                }
              >
                <option value="csv">接口清单（CSV）</option>
                <option value="csv_params">参数明细（CSV）</option>
                <option value="json">能力快照（JSON）</option>
              </select>
              {exportFormat === "csv" ? (
                <label className="flex items-center gap-1">
                  <input
                    type="checkbox"
                    checked={includeSchema}
                    onChange={(event) => setIncludeSchema(event.target.checked)}
                  />
                  包含完整 Schema
                </label>
              ) : null}
            </div>
          }
        />
      ) : null}
    </section>
  );
}
