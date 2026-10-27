/** 与 Rust 侧契约一一对应 */

/** 应用信息（get_app_info） */
export interface AppInfo {
  version: string;
  data_dir: string;
  log_dir: string;
  supported_protocol_versions: string[];
  /** 便携模式：数据在 exe 同目录 data/ */
  portable: boolean;
  /** 数据目录来源：env / portable / appdata / fallback */
  data_dir_source: string;
  /** 启动配置文件路径（未找到时为 null） */
  config_path: string | null;
  /** not_found / pending / applied / partial / error */
  config_status: string;
  config_error: string | null;
  /** 配置文件在 seed 后是否变化（seed 模式不会自动覆盖） */
  config_changed: boolean;
  default_server_id: string | null;
  auto_connect: boolean;
}

/** 服务器来源（配置文件 seed） */
export interface ServerOrigin {
  source: string;
  key: string;
}

export type TransportKind = "stdio" | "streamable_http" | "sse";

export interface KeyValue {
  key: string;
  value: string | null;
  secret: boolean;
}

export interface StdioConfig {
  command: string;
  args: string[];
  env: KeyValue[];
  cwd: string | null;
  shell_wrap: "resolve" | "cmd_shell" | "never";
}

/** OAuth 授权请求额外参数（v0.3.1） */
export interface OAuthParam {
  key: string;
  value: string;
}

/** OAuth 2.1 登录配置（v0.3.0；可选字段兼容后端省略） */
export interface OAuthConfig {
  scopes: string[];
  client_id?: string | null;
  /** UI 掩码；真实 secret 由后端 oauth_credentials 保存 */
  client_secret?: string | null;
  /** 完整回调地址；缺省 http://127.0.0.1:33318/oauth/callback */
  redirect_uri?: string | null;
  authorization_endpoint?: string | null;
  token_endpoint?: string | null;
  registration_endpoint?: string | null;
  issuer?: string | null;
  allow_missing_issuer?: boolean;
  /** 授权请求额外参数（v0.3.1） */
  authorize_params?: OAuthParam[];
  /** 显式允许自定义 header 发往跨源授权服务器（v0.3.1） */
  forward_headers_to_auth_server?: boolean;
  /** CIMD 客户端元数据文档 URL（v0.3.1） */
  client_metadata_url?: string | null;
  /** 手工端点 token 认证方式（v0.3.1） */
  token_endpoint_auth_method?: string | null;
  /** 退出登录撤销端点（v0.3.1） */
  revocation_endpoint?: string | null;
}

export interface HttpConfig {
  url: string;
  headers: KeyValue[];
  timeout_secs: number | null;
  allow_insecure_tls: boolean;
  /** Some = OAuth；缺省 = 手工 Authorization / 无认证 */
  oauth?: OAuthConfig | null;
}

export interface ServerConfig {
  id: string;
  name: string;
  transport: TransportKind;
  stdio: StdioConfig | null;
  http: HttpConfig | null;
  protocol_version_pin: string | null;
  enabled: boolean;
  tags: string[];
  /** 配置文件来源；手工添加为 null/缺省 */
  origin?: ServerOrigin | null;
  created_at: string;
  updated_at: string;
  last_connected_at: string | null;
}

export type McpError = {
  kind: string;
  message?: string;
} & Record<string, unknown>;

export type ConnectionState =
  | { state: "disconnected" }
  | { state: "connecting" }
  | { state: "ready"; handshake: Handshake }
  | { state: "degraded"; reason: string }
  | { state: "lost"; reason: string; can_reconnect: boolean }
  | { state: "error"; error: McpError };

export interface Handshake {
  server_name: string;
  server_version: string;
  protocol_version: string;
  lifecycle: "modern" | "legacy";
  capabilities: unknown;
  instructions: string | null;
}

export interface ServerOAuthSummary {
  configured: boolean;
  authorized: boolean;
  refreshable: boolean;
  scopes: string[];
  expires_at: string | null;
  /** 403 insufficient_scope 缺少的 scope（v0.3.1） */
  required_scope?: string | null;
  /** 连接仍 Ready，但需要补充授权（v0.3.1） */
  step_up_required?: boolean;
}

export interface ServerSummary {
  id: string;
  name: string;
  transport: TransportKind;
  enabled: boolean;
  tags: string[];
  origin?: ServerOrigin | null;
  oauth?: ServerOAuthSummary | null;
  last_connected_at: string | null;
  state: ConnectionState;
}

export type SecretKindInput = "env" | "header" | "oauth";
export type SecretAction = "keep" | "replace" | "clear";

export interface SecretInput {
  kind: SecretKindInput;
  key: string;
  action: SecretAction;
  value?: string | null;
}

export interface ServerDraft {
  name: string;
  transport: TransportKind;
  stdio: StdioConfig | null;
  http: HttpConfig | null;
  protocol_version_pin: string | null;
  enabled: boolean;
  tags: string[];
  secrets: SecretInput[];
}

export interface ToolInfo {
  name: string;
  title: string | null;
  description: string | null;
  input_schema: unknown | null;
  output_schema: unknown | null;
  annotations: unknown | null;
}

export interface ResourceInfo {
  uri: string;
  name: string;
  title: string | null;
  description: string | null;
  mime_type: string | null;
  size: number | null;
  annotations: unknown | null;
}

export interface ResourceTemplateInfo {
  uri_template: string;
  name: string;
  title: string | null;
  description: string | null;
  mime_type: string | null;
}

export interface PromptInfo {
  name: string;
  title: string | null;
  description: string | null;
  arguments: { name: string; description: string | null; required: boolean }[];
}

export interface CapabilitySnapshot {
  server_id: string;
  tools: ToolInfo[];
  resources: ResourceInfo[];
  resource_templates: ResourceTemplateInfo[];
  prompts: PromptInfo[];
  fetched_at: string;
  stale: boolean;
  /** 枚举降级提示（如可选方法 -32601） */
  warnings: string[];
}

export type CallMethod =
  "tools_call" | "resources_read" | "prompts_get" | "probe";
export type CallStatus = "ok" | "error" | "canceled";
export type CallOrigin = "manual" | "test";

export interface CallError {
  kind: string;
  message?: string;
  code?: number;
  data?: unknown;
  timeout_secs?: number;
  reason?: string;
  capability?: string;
  /** v0.3.1：OAuth 挑战原文 / 403 详情 */
  detail?: string;
  required_scope?: string;
  resource_metadata_url?: string;
  /** 非 step-up 的 403 */
  status?: number;
}

/** 单次调用的原始 JSON-RPC 帧（脱敏/截断前采集） */
export interface CallFrames {
  /** 线上 JSON-RPC id，保留 number / string 类型 */
  rpc_id: string | number | null;
  request: unknown | null;
  response: unknown | null;
  /** 任一帧因超限/异常未保存 */
  truncated: boolean;
}

export interface CallRecord {
  id: string;
  server_id: string;
  server_name: string;
  method: CallMethod;
  target: string;
  params: unknown;
  result: unknown | null;
  error: CallError | null;
  status: CallStatus;
  duration_ms: number;
  protocol_version: string;
  transport: TransportKind;
  truncated: boolean;
  size_bytes: number;
  origin: CallOrigin;
  /** 原始帧；旧记录缺省或为 null */
  frames?: CallFrames | null;
  created_at: string;
}

export interface ProbeResult {
  ok: boolean;
  rtt_ms: number;
  detail: string;
}

export interface CallRecordSummary {
  id: string;
  server_id: string;
  server_name: string;
  method: CallMethod;
  target: string;
  status: CallStatus;
  duration_ms: number;
  size_bytes: number;
  truncated: boolean;
  origin: CallOrigin;
  created_at: string;
}

/** 参数信封：工具/提示词为 { name, arguments }，资源为 { uri } */
export type CallParams = Record<string, unknown>;

/** 接口页“最近参数”列表项（不含完整参数） */
export interface RecentCallParam {
  id: string;
  created_at: string;
  status: CallStatus;
  duration_ms: number;
  origin: CallOrigin;
  preview: string;
  param_count: number;
  repeat_count: number;
  params_bytes: number;
}
