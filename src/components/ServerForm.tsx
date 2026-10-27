import { useState } from "react";
import { ErrorDetailDialog } from "./ErrorDetailDialog";
import { JsonViewer } from "./JsonViewer";
import { toErrorInfo, type ErrorInfo } from "../lib/errors";
import { toast } from "../stores/toast";
import type { ConnectionTestReport } from "../api/servers";
import type {
  HttpConfig,
  KeyValue,
  OAuthConfig,
  OAuthParam,
  SecretInput,
  ServerConfig,
  ServerDraft,
  StdioConfig,
  TransportKind,
} from "../types/models";

type AuthMode = "none" | "header" | "oauth";

function isAuthorizationHeader(item: KeyValue): boolean {
  return item.key.trim().toLowerCase() === "authorization";
}

function initialAuthMode(initial?: ServerConfig | null): AuthMode {
  if (initial?.http?.oauth) return "oauth";
  if (initial?.http?.headers.some(isAuthorizationHeader)) return "header";
  return "none";
}

function parseScopes(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((scope) => scope.trim())
    .filter(Boolean);
}

/** 授权参数文本：一行一个 key=value；按第一个 = 切分，空行/# 注释忽略 */
function parseAuthorizeParams(text: string): {
  params: OAuthParam[];
  error: string | null;
} {
  const params: OAuthParam[] = [];
  const seen = new Set<string>();
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const index = line.indexOf("=");
    if (index <= 0) {
      return { params: [], error: `授权参数「${line}」缺少 key=value 格式` };
    }
    const key = line.slice(0, index).trim();
    const value = line.slice(index + 1).trim();
    if (!key) {
      return { params: [], error: `授权参数「${line}」的 key 不能为空` };
    }
    const lower = key.toLowerCase();
    if (seen.has(lower)) {
      return { params: [], error: `授权参数「${key}」重复` };
    }
    seen.add(lower);
    params.push({ key, value });
  }
  return { params, error: null };
}

function formatAuthorizeParams(params: OAuthParam[] | undefined): string {
  return (params ?? [])
    .map((param) => `${param.key}=${param.value}`)
    .join("\n");
}

const TRANSPORTS: { value: TransportKind; label: string }[] = [
  { value: "stdio", label: "stdio（本地进程）" },
  { value: "streamable_http", label: "Streamable HTTP" },
  { value: "sse", label: "遗留 HTTP+SSE（旧协议）" },
];

/** 本地地址未显式指定端口时给出提示（v0.1.2；对应真实案例：URL 漏写 :8000） */
function needsPortHint(url: string): boolean {
  const match = url.match(/^https?:\/\/([^/?#]+)/i);
  if (!match) return false;
  const hostPort = match[1];
  const host = hostPort.split(":")[0].toLowerCase();
  if (host !== "localhost" && host !== "127.0.0.1") return false;
  return !hostPort.includes(":");
}

/** 键值行编辑器（env / headers 共用；可标记敏感）
 *
 * v0.1.2：敏感值若为掩码形态（`****`），输入框显示为空并以占位符提示
 * "已配置"——"留空 = 不修改"语义直观化，避免把掩码误当真实值或在其后追加输入。
 */
function KeyValueEditor({
  items,
  onChange,
  placeholderKey,
}: {
  items: KeyValue[];
  onChange: (items: KeyValue[]) => void;
  placeholderKey: string;
}) {
  return (
    <div className="space-y-1">
      {items.map((item, index) => {
        const masked = item.secret && (item.value ?? "").includes("****");
        const placeholder = item.secret
          ? masked
            ? "已配置（留空=不修改；输入即替换）"
            : "敏感值（留空表示不修改）"
          : "值";
        return (
          <div key={index} className="flex items-center gap-2">
            <input
              className="w-40 rounded border border-slate-200 px-2 py-1 text-xs"
              placeholder={placeholderKey}
              value={item.key}
              onChange={(e) => {
                const next = [...items];
                next[index] = { ...item, key: e.target.value };
                onChange(next);
              }}
            />
            <input
              className="flex-1 rounded border border-slate-200 px-2 py-1 text-xs"
              placeholder={placeholder}
              value={masked ? "" : (item.value ?? "")}
              onChange={(e) => {
                const next = [...items];
                next[index] = { ...item, value: e.target.value };
                onChange(next);
              }}
            />
            <label className="flex items-center gap-1 text-xs text-slate-500">
              <input
                type="checkbox"
                checked={item.secret}
                onChange={(e) => {
                  const next = [...items];
                  next[index] = { ...item, secret: e.target.checked };
                  onChange(next);
                }}
              />
              敏感
            </label>
            <button
              type="button"
              className="text-xs text-slate-400 hover:text-red-500"
              onClick={() => onChange(items.filter((_, i) => i !== index))}
            >
              删除
            </button>
          </div>
        );
      })}
      <button
        type="button"
        className="text-xs text-slate-500 hover:underline"
        onClick={() =>
          onChange([...items, { key: "", value: "", secret: false }])
        }
      >
        + 添加
      </button>
    </div>
  );
}

/** 服务器表单（新建 / 编辑） */
export function ServerForm({
  initial,
  onSubmit,
  onCancel,
  onTest,
  submitting,
  plaintextCredentials,
}: {
  initial?: ServerConfig | null;
  onSubmit: (draft: ServerDraft) => void;
  onCancel: () => void;
  /** 测试连接（v0.1.1；未保存草稿可用） */
  onTest?: (draft: ServerDraft) => Promise<ConnectionTestReport>;
  submitting?: boolean;
  /** 便携模式或配置文件来源：凭据明文保存在本地数据库 */
  plaintextCredentials?: boolean;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [transport, setTransport] = useState<TransportKind>(
    initial?.transport ?? "stdio",
  );
  const [pin, setPin] = useState(initial?.protocol_version_pin ?? "");
  const [stdio, setStdio] = useState<StdioConfig>(
    initial?.stdio ?? {
      command: "",
      args: [],
      env: [],
      cwd: null,
      shell_wrap: "resolve",
    },
  );
  const [http, setHttp] = useState<HttpConfig>(
    initial?.http ?? {
      url: "",
      headers: [],
      timeout_secs: null,
      allow_insecure_tls: false,
    },
  );
  const [argsText, setArgsText] = useState(
    (initial?.stdio?.args ?? []).join("\n"),
  );
  const [authMode, setAuthMode] = useState<AuthMode>(initialAuthMode(initial));
  const [scopesText, setScopesText] = useState(
    (initial?.http?.oauth?.scopes ?? []).join(" "),
  );
  const [clientId, setClientId] = useState(
    initial?.http?.oauth?.client_id ?? "",
  );
  const [clientSecret, setClientSecret] = useState("");
  const [clearClientSecret, setClearClientSecret] = useState(false);
  const [redirectUri, setRedirectUri] = useState(
    initial?.http?.oauth?.redirect_uri ?? "",
  );
  const [authorizationEndpoint, setAuthorizationEndpoint] = useState(
    initial?.http?.oauth?.authorization_endpoint ?? "",
  );
  const [tokenEndpoint, setTokenEndpoint] = useState(
    initial?.http?.oauth?.token_endpoint ?? "",
  );
  const [registrationEndpoint, setRegistrationEndpoint] = useState(
    initial?.http?.oauth?.registration_endpoint ?? "",
  );
  const [oauthIssuer, setOauthIssuer] = useState(
    initial?.http?.oauth?.issuer ?? "",
  );
  const [allowMissingIssuer, setAllowMissingIssuer] = useState(
    initial?.http?.oauth?.allow_missing_issuer ?? false,
  );
  const [authorizeParamsText, setAuthorizeParamsText] = useState(
    formatAuthorizeParams(initial?.http?.oauth?.authorize_params),
  );
  const [forwardHeadersToAuthServer, setForwardHeadersToAuthServer] = useState(
    initial?.http?.oauth?.forward_headers_to_auth_server ?? false,
  );
  const [clientMetadataUrl, setClientMetadataUrl] = useState(
    initial?.http?.oauth?.client_metadata_url ?? "",
  );
  const [tokenEndpointAuthMethod, setTokenEndpointAuthMethod] = useState(
    initial?.http?.oauth?.token_endpoint_auth_method ?? "",
  );
  const [revocationEndpoint, setRevocationEndpoint] = useState(
    initial?.http?.oauth?.revocation_endpoint ?? "",
  );
  const [error, setError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [testReport, setTestReport] = useState<ConnectionTestReport | null>(
    null,
  );
  const [testDetail, setTestDetail] = useState<ErrorInfo | null>(null);

  /** 校验并构建草稿（保存 / 测试连接共用） */
  function buildDraft(): ServerDraft | null {
    setError(null);
    if (!name.trim()) {
      setError("服务器名称不能为空");
      return null;
    }
    const isStdio = transport === "stdio";
    const isOauth = !isStdio && authMode === "oauth";
    if (isStdio && !stdio.command.trim()) {
      setError("命令不能为空");
      return null;
    }
    if (!isStdio && !http.url.startsWith("http")) {
      setError("URL 必须以 http:// 或 https:// 开头");
      return null;
    }
    if (isOauth && transport === "sse") {
      setError("OAuth 仅支持 Streamable HTTP，不支持遗留 SSE");
      return null;
    }
    if (isOauth && !clientId.trim() && clientSecret.trim() === "") {
      // 允许 DCR；此处仅提示，不强制
    }

    // v0.1.1：掩码值（含 "****"）视为"未修改"，按 keep 提交——
    // 防止把回显掩码误写回凭据库导致真实 token 丢失（见 01 文档根因 B）
    const secrets: SecretInput[] = [
      ...(isStdio ? stdio.env : http.headers)
        .filter((kv) => kv.secret || kv.value)
        .map((kv) => {
          const value = kv.value ?? "";
          const masked = value.includes("****");
          const action =
            !value || masked ? ("keep" as const) : ("replace" as const);
          return {
            kind: isStdio ? ("env" as const) : ("header" as const),
            key: kv.key,
            action,
            value: action === "replace" ? value : null,
          };
        }),
    ];
    if (isOauth) {
      secrets.push({
        kind: "oauth",
        key: "client_secret",
        action: clearClientSecret
          ? "clear"
          : clientSecret.trim()
            ? "replace"
            : "keep",
        value: clearClientSecret ? null : clientSecret.trim() || null,
      });
    } else if (initial?.http?.oauth) {
      // 从 OAuth 切回手工/无认证：清理旧客户端密钥
      secrets.push({
        kind: "oauth",
        key: "client_secret",
        action: "clear",
        value: null,
      });
    }

    const effectiveHeaders =
      authMode === "header"
        ? http.headers
        : http.headers.filter((item) => !isAuthorizationHeader(item));

    const authorizeParams = parseAuthorizeParams(authorizeParamsText);
    if (isOauth && authorizeParams.error) {
      setError(authorizeParams.error);
      return null;
    }

    const oauth: OAuthConfig | null = isOauth
      ? {
          scopes: parseScopes(scopesText),
          client_id: clientId.trim() || null,
          client_secret: null,
          redirect_uri: redirectUri.trim() || null,
          authorization_endpoint: authorizationEndpoint.trim() || null,
          token_endpoint: tokenEndpoint.trim() || null,
          registration_endpoint: registrationEndpoint.trim() || null,
          issuer: oauthIssuer.trim() || null,
          allow_missing_issuer: allowMissingIssuer,
          authorize_params: authorizeParams.params,
          forward_headers_to_auth_server: forwardHeadersToAuthServer,
          client_metadata_url: clientMetadataUrl.trim() || null,
          token_endpoint_auth_method: tokenEndpointAuthMethod || null,
          revocation_endpoint: revocationEndpoint.trim() || null,
        }
      : null;

    return {
      name: name.trim(),
      transport,
      stdio: isStdio
        ? {
            ...stdio,
            args: argsText
              .split("\n")
              .map((line) => line.trim())
              .filter((line) => line.length > 0),
          }
        : null,
      http: isStdio
        ? null
        : { ...http, headers: effectiveHeaders, oauth },
      protocol_version_pin: pin.trim() ? pin.trim() : null,
      enabled: true,
      tags: [],
      secrets,
    };
  }

  function submit() {
    const draft = buildDraft();
    if (!draft) return;
    onSubmit(draft);
  }

  async function runTest() {
    if (!onTest) return;
    const draft = buildDraft();
    if (!draft) return;
    setTesting(true);
    setTestReport(null);
    try {
      const report = await onTest(draft);
      setTestReport(report);
    } catch (err) {
      toast.fromError(err, "测试连接失败");
    } finally {
      setTesting(false);
    }
  }

  const isStdio = transport === "stdio";

  return (
    <div className="space-y-4 rounded-lg border border-slate-200 bg-white p-4">
      {plaintextCredentials ? (
        <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1 text-[11px] text-amber-700">
          便携/配置文件来源：请求头与环境变量以明文保存在本地数据库（导出时敏感项仍会脱敏）。
        </div>
      ) : null}
      <div className="grid grid-cols-2 gap-4">
        <label className="space-y-1 text-xs text-slate-600">
          名称
          <input
            className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="如：本地 filesystem"
          />
        </label>
        <label className="space-y-1 text-xs text-slate-600">
          传输类型
          <select
            className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
            value={transport}
            onChange={(e) => {
              const next = e.target.value as TransportKind;
              setTransport(next);
              if (next === "stdio") setAuthMode("none");
              else if (next === "sse" && authMode === "oauth") {
                setAuthMode("header");
              }
            }}
          >
            {TRANSPORTS.map((item) => (
              <option key={item.value} value={item.value}>
                {item.label}
              </option>
            ))}
          </select>
        </label>
      </div>

      {isStdio ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-4">
            <label className="space-y-1 text-xs text-slate-600">
              命令
              <input
                className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                value={stdio.command}
                onChange={(e) =>
                  setStdio({ ...stdio, command: e.target.value })
                }
                placeholder="npx / uvx / 可执行文件路径"
              />
            </label>
            <label className="space-y-1 text-xs text-slate-600">
              工作目录（可选）
              <input
                className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                value={stdio.cwd ?? ""}
                onChange={(e) =>
                  setStdio({ ...stdio, cwd: e.target.value || null })
                }
              />
            </label>
          </div>
          <label className="block space-y-1 text-xs text-slate-600">
            参数（每行一个）
            <textarea
              className="h-20 w-full rounded border border-slate-200 p-2 font-mono text-xs"
              value={argsText}
              onChange={(e) => setArgsText(e.target.value)}
              placeholder={"-y\n@modelcontextprotocol/server-everything"}
            />
          </label>
          <div className="space-y-1 text-xs text-slate-600">
            环境变量
            <KeyValueEditor
              items={stdio.env}
              onChange={(env) => setStdio({ ...stdio, env })}
              placeholderKey="名称，如 API_KEY"
            />
          </div>
          <label className="space-y-1 text-xs text-slate-600">
            命令解析方式
            <select
              className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
              value={stdio.shell_wrap}
              onChange={(e) =>
                setStdio({
                  ...stdio,
                  shell_wrap: e.target.value as StdioConfig["shell_wrap"],
                })
              }
            >
              <option value="resolve">
                resolve（推荐：解析全路径后直接启动）
              </option>
              <option value="cmd_shell">
                cmd_shell（经 cmd /C，存在转义风险）
              </option>
              <option value="never">never（原样启动，排障用）</option>
            </select>
          </label>
        </div>
      ) : (
        <div className="space-y-3">
          <label className="block space-y-1 text-xs text-slate-600">
            MCP 端点 URL
            <input
              className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
              value={http.url}
              onChange={(e) => setHttp({ ...http, url: e.target.value })}
              placeholder="https://example.com/mcp 或 http://127.0.0.1:8080/sse"
            />
          </label>
          {needsPortHint(http.url) ? (
            <div className="text-[11px] text-amber-600">
              提示：该本地地址未指定端口（将使用默认端口 80）。本地 MCP
              服务通常带自定义端口， 如 http://127.0.0.1:8000/mcp
            </div>
          ) : null}
          <label className="block space-y-1 text-xs text-slate-600">
            认证方式
            <select
              className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
              value={authMode}
              onChange={(e) => {
                const mode = e.target.value as AuthMode;
                setAuthMode(mode);
                if (mode !== "header") {
                  setHttp((prev) => ({
                    ...prev,
                    headers: prev.headers.filter(
                      (item) => !isAuthorizationHeader(item),
                    ),
                  }));
                }
                if (mode !== "oauth" && initial?.http?.oauth) {
                  setClearClientSecret(true);
                }
              }}
            >
              <option value="none">无认证</option>
              <option value="header">Authorization 请求头</option>
              <option value="oauth" disabled={transport === "sse"}>
                OAuth 2.1 / OAuth 2.0 登录（授权码 + PKCE，仅 Streamable HTTP）
              </option>
            </select>
          </label>

          {authMode === "oauth" ? (
            <div className="space-y-3 rounded-lg border border-sky-200 bg-sky-50/40 p-3">
              <div className="grid grid-cols-2 gap-4">
                <label className="space-y-1 text-xs text-slate-600">
                  scopes（空格/逗号分隔，留空=自动）
                  <input
                    className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                    value={scopesText}
                    onChange={(e) => setScopesText(e.target.value)}
                    placeholder="如 openid profile"
                  />
                </label>
                <label className="space-y-1 text-xs text-slate-600">
                  client_id（留空=动态注册）
                  <input
                    className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                    value={clientId}
                    onChange={(e) => setClientId(e.target.value)}
                  />
                </label>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <label className="space-y-1 text-xs text-slate-600">
                  回调地址（需与授权服务器登记一致）
                  <input
                    className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                    value={redirectUri}
                    onChange={(e) => setRedirectUri(e.target.value)}
                    placeholder="http://127.0.0.1:33318/oauth/callback"
                  />
                </label>
                <label className="space-y-1 text-xs text-slate-600">
                  client_secret（留空=不修改）
                  <input
                    type="password"
                    className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                    value={clientSecret}
                    onChange={(e) => {
                      setClientSecret(e.target.value);
                      if (e.target.value) setClearClientSecret(false);
                    }}
                    placeholder={
                      initial?.http?.oauth?.client_secret
                        ? "已配置（留空=不修改；输入即替换）"
                        : "仅预注册机密客户端需要"
                    }
                    disabled={clearClientSecret}
                  />
                </label>
              </div>
              {initial?.http?.oauth?.client_secret ? (
                <label className="flex items-center gap-2 text-xs text-slate-600">
                  <input
                    type="checkbox"
                    checked={clearClientSecret}
                    onChange={(e) => {
                      setClearClientSecret(e.target.checked);
                      if (e.target.checked) setClientSecret("");
                    }}
                  />
                  清除客户端密钥
                </label>
              ) : null}
              <details className="rounded border border-slate-200 bg-white/70 px-2 py-1">
                <summary className="cursor-pointer text-xs text-slate-500">
                  高级：手工端点 / issuer
                </summary>
                <div className="mt-2 grid grid-cols-2 gap-3">
                  <label className="space-y-1 text-xs text-slate-600">
                    authorization_endpoint
                    <input
                      className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                      value={authorizationEndpoint}
                      onChange={(e) => setAuthorizationEndpoint(e.target.value)}
                    />
                  </label>
                  <label className="space-y-1 text-xs text-slate-600">
                    token_endpoint
                    <input
                      className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                      value={tokenEndpoint}
                      onChange={(e) => setTokenEndpoint(e.target.value)}
                    />
                  </label>
                  <label className="space-y-1 text-xs text-slate-600">
                    registration_endpoint
                    <input
                      className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                      value={registrationEndpoint}
                      onChange={(e) => setRegistrationEndpoint(e.target.value)}
                    />
                  </label>
                  <label className="space-y-1 text-xs text-slate-600">
                    issuer
                    <input
                      className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                      value={oauthIssuer}
                      onChange={(e) => setOauthIssuer(e.target.value)}
                    />
                  </label>
                </div>
                <label className="mt-2 flex items-center gap-2 text-xs text-slate-600">
                  <input
                    type="checkbox"
                    checked={allowMissingIssuer}
                    onChange={(e) => setAllowMissingIssuer(e.target.checked)}
                  />
                  容忍元数据缺少 issuer（仅不规范服务器）
                </label>
              </details>
              <details className="rounded border border-slate-200 bg-white/70 px-2 py-1">
                <summary className="cursor-pointer text-xs text-slate-500">
                  高级：授权请求参数 / OAuth 请求头
                </summary>
                <label className="mt-2 block space-y-1 text-xs text-slate-600">
                  授权请求额外参数（一行一个 key=value，空行和 # 注释忽略）
                  <textarea
                    className="h-20 w-full resize-y rounded border border-slate-200 px-2 py-1 font-mono text-[11px]"
                    value={authorizeParamsText}
                    onChange={(e) => setAuthorizeParamsText(e.target.value)}
                    placeholder={"audience=https://api.example.com\naccess_type=offline\nprompt=consent"}
                  />
                </label>
                <label className="mt-2 flex items-start gap-2 text-xs text-slate-600">
                  <input
                    type="checkbox"
                    className="mt-0.5"
                    checked={forwardHeadersToAuthServer}
                    onChange={(e) =>
                      setForwardHeadersToAuthServer(e.target.checked)
                    }
                  />
                  <span>
                    把自定义请求头发往跨源授权服务器（默认只发同源；开启后可能把网关密钥发给第三方 IdP）
                  </span>
                </label>
                <label className="mt-2 block space-y-1 text-xs text-slate-600">
                  client_metadata_url（CIMD，HTTPS；留空=动态注册）
                  <input
                    className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                    value={clientMetadataUrl}
                    onChange={(e) => setClientMetadataUrl(e.target.value)}
                    placeholder="https://client.example.com/oauth-client-metadata.json"
                  />
                </label>
                <label className="mt-2 block space-y-1 text-xs text-slate-600">
                  手工端点 token 认证方式
                  <select
                    className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                    value={tokenEndpointAuthMethod}
                    onChange={(e) => setTokenEndpointAuthMethod(e.target.value)}
                  >
                    <option value="">默认（client_secret_basic）</option>
                    <option value="client_secret_basic">client_secret_basic</option>
                    <option value="client_secret_post">client_secret_post</option>
                    <option value="none">none（公共客户端）</option>
                  </select>
                </label>
                <label className="mt-2 block space-y-1 text-xs text-slate-600">
                  revocation_endpoint（退出登录时撤销授权，可选）
                  <input
                    className="w-full rounded border border-slate-200 px-2 py-1 text-xs"
                    value={revocationEndpoint}
                    onChange={(e) => setRevocationEndpoint(e.target.value)}
                    placeholder="https://auth.example.com/revoke"
                  />
                </label>
                <p className="mt-1 text-[11px] text-slate-500">
                  已用 DCR/CIMD 登录时，切换注册方式请先在服务器卡片点“清除注册”。
                </p>
              </details>
              <p className="text-[11px] text-slate-500">
                OAuth 模式会忽略 Authorization 请求头；其他自定义请求头默认只发往 MCP 服务器同源地址。
              </p>
            </div>
          ) : (
            <div className="space-y-1 text-xs text-slate-600">
              请求头（Authorization 填「Bearer xxx」或仅 token
              均可，建议勾选"敏感"入凭据库）
              <KeyValueEditor
                items={http.headers}
                onChange={(headers) => setHttp({ ...http, headers })}
                placeholderKey="如 Authorization"
              />
            </div>
          )}

          {authMode === "oauth" ? (
            <div className="space-y-1 text-xs text-slate-600">
              其他请求头
              <KeyValueEditor
                items={http.headers.filter(
                  (item) => !isAuthorizationHeader(item),
                )}
                onChange={(headers) => setHttp({ ...http, headers })}
                placeholderKey="如 X-Trace"
              />
            </div>
          ) : null}
          <div className="grid grid-cols-2 gap-4">
            <label className="space-y-1 text-xs text-slate-600">
              超时覆盖（秒，留空=全局）
              <input
                type="number"
                className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
                value={http.timeout_secs ?? ""}
                onChange={(e) =>
                  setHttp({
                    ...http,
                    timeout_secs: e.target.value
                      ? Number(e.target.value)
                      : null,
                  })
                }
              />
            </label>
            <label className="flex items-center gap-2 self-end text-xs text-slate-600">
              <input
                type="checkbox"
                checked={http.allow_insecure_tls}
                onChange={(e) =>
                  setHttp({ ...http, allow_insecure_tls: e.target.checked })
                }
              />
              允许不安全 TLS（仅本地自签证书；有安全风险）
            </label>
          </div>
        </div>
      )}

      <div className="grid grid-cols-2 gap-4">
        <label className="space-y-1 text-xs text-slate-600">
          协议版本（高级，留空=自动协商）
          <select
            className="w-full rounded border border-slate-200 px-2 py-1 text-sm"
            value={pin}
            onChange={(e) => setPin(e.target.value)}
          >
            <option value="">自动协商（推荐）</option>
            <option value="2026-07-28">2026-07-28（现代世代）</option>
            <option value="2025-11-25">2025-11-25（旧世代握手）</option>
          </select>
        </label>
      </div>

      {error ? <div className="text-xs text-red-500">{error}</div> : null}

      {testReport ? (
        <div
          className={`space-y-2 rounded-lg border p-3 ${
            testReport.ok
              ? "border-emerald-200 bg-emerald-50"
              : "border-red-200 bg-red-50"
          }`}
        >
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span
              className={testReport.ok ? "text-emerald-700" : "text-red-600"}
            >
              {testReport.ok ? "✓ 连接测试通过" : "✗ 连接测试失败"}
            </span>
            {testReport.ok && testReport.handshake ? (
              <span className="text-slate-500">
                {testReport.handshake.server_name} v
                {testReport.handshake.server_version} · 协议{" "}
                {testReport.handshake.protocol_version}（
                {testReport.handshake.lifecycle === "modern"
                  ? "现代世代"
                  : "旧世代"}
                ）· 工具 {testReport.tool_count} · 资源{" "}
                {testReport.resource_count} · 模板 {testReport.template_count} ·
                提示词 {testReport.prompt_count}
              </span>
            ) : null}
            {!testReport.ok && testReport.error ? (
              <button
                type="button"
                className="rounded border border-red-200 bg-white px-2 py-0.5 text-red-600 hover:bg-red-50"
                onClick={() => setTestDetail(toErrorInfo(testReport.error))}
              >
                查看详情
              </button>
            ) : null}
          </div>
          <ul className="space-y-1">
            {testReport.steps.map((step) => (
              <li key={step.name} className="flex items-start gap-2 text-xs">
                <span className={step.ok ? "text-emerald-600" : "text-red-500"}>
                  {step.ok ? "✓" : "✗"}
                </span>
                <span className="w-28 shrink-0 text-slate-700">
                  {step.name}
                </span>
                <span className="w-14 shrink-0 text-slate-400">
                  {step.elapsed_ms}ms
                </span>
                <span className="min-w-0 break-all text-slate-500">
                  {step.detail}
                </span>
              </li>
            ))}
          </ul>
          {testReport.wire.length > 0 ? (
            <details className="rounded border border-slate-200 bg-white/70 px-2 py-1">
              <summary className="cursor-pointer text-xs text-slate-500">
                查看本次报文（{testReport.wire.length} 条）
              </summary>
              <div className="mt-2 max-h-64 overflow-auto">
                <JsonViewer value={testReport.wire} defaultMode="tree" />
              </div>
            </details>
          ) : null}
        </div>
      ) : null}

      <div className="flex items-center gap-2">
        <button
          type="button"
          className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white hover:bg-slate-700 disabled:opacity-50"
          onClick={submit}
          disabled={submitting || testing}
        >
          {submitting ? "保存中…" : "保存"}
        </button>
        {onTest ? (
          <button
            type="button"
            className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50 disabled:opacity-50"
            onClick={() => void runTest()}
            disabled={testing || submitting}
          >
            {testing ? "测试中…" : "测试连接"}
          </button>
        ) : null}
        <button
          type="button"
          className="rounded border border-slate-200 px-3 py-1.5 text-sm text-slate-600 hover:bg-slate-50"
          onClick={onCancel}
        >
          取消
        </button>
        <span className="text-[11px] text-slate-400">
          测试连接仅验证配置（不保存、不记录历史）
        </span>
      </div>

      {testDetail ? (
        <ErrorDetailDialog
          info={testDetail}
          onClose={() => setTestDetail(null)}
        />
      ) : null}
    </div>
  );
}
