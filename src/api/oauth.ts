import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 登录开始结果（Rust `OAuthLoginStart`） */
export interface OAuthLoginStart {
  flow_id: string;
  auth_url: string;
  redirect_uri: string;
  flow_timeout_secs: number;
  browser_opened: boolean;
  open_error: string | null;
  /** localhost/v6 等回调提示（不阻断登录） */
  callback_warning?: string | null;
  /** 最终授权 URL 中的 scope；空 = 由授权服务器决定 */
  scopes: string[];
}

/** 登录状态（Rust `OAuthStatus`） */
export interface OAuthStatus {
  configured: boolean;
  authorized: boolean;
  refreshable: boolean;
  client_id: string | null;
  registration?: "preregistered" | "dcr" | "cimd" | null;
  client_metadata_url?: string | null;
  token_endpoint_auth_method?: string | null;
  scopes: string[];
  expires_at: string | null;
  issuer: string | null;
}

/** `mcp://oauth` 事件载荷 */
export interface OAuthPushPayload {
  server_id: string;
  flow_id: string;
  state: "authorized" | "error" | "canceled";
  message?: string | null;
}

/** 开始浏览器 OAuth 登录（后端会尝试打开系统浏览器） */
export function startOAuthLogin(serverId: string): Promise<OAuthLoginStart> {
  return invoke<OAuthLoginStart>("start_oauth_login", { serverId });
}

/** 取消登录（按 flow_id；幂等） */
export function cancelOAuthLogin(flowId: string): Promise<void> {
  return invoke<void>("cancel_oauth_login", { flowId });
}

/** 查询服务器 OAuth 状态 */
export function getOAuthStatus(serverId: string): Promise<OAuthStatus> {
  return invoke<OAuthStatus>("get_oauth_status", { serverId });
}

export interface OAuthLogoutResult {
  revoked: boolean;
  detail?: string | null;
}

export interface OAuthDiagnostics {
  configured: boolean;
  authorized: boolean;
  refreshable: boolean;
  client_id: string | null;
  client_metadata_url: string | null;
  registration: "preregistered" | "dcr" | "cimd" | null;
  token_endpoint_auth_method: string | null;
  issuer: string | null;
  authorization_endpoint: string | null;
  token_endpoint: string | null;
  registration_endpoint: string | null;
  revocation_endpoint: string | null;
  resource: string;
  scopes_supported: string[];
  granted_scopes: string[];
  code_challenge_methods_supported: string[];
  token_endpoint_auth_methods_supported: string[];
  revocation_endpoint_auth_methods_supported: string[];
  expires_at: string | null;
  discovery_error?: string | null;
  metadata?: unknown;
}

export interface OAuthAccessToken {
  access_token: string;
  token_type: string;
  expires_at: string | null;
}

/** OAuth 诊断（显式 discovery；不返回 secret/token） */
export function getOAuthDiagnostics(
  serverId: string,
): Promise<OAuthDiagnostics> {
  return invoke<OAuthDiagnostics>("get_oauth_diagnostics", { serverId });
}

/** 强制刷新 OAuth token */
export function refreshOAuthToken(serverId: string): Promise<OAuthStatus> {
  return invoke<OAuthStatus>("refresh_oauth_token", { serverId });
}

/** 查看当前 access token（显式用户操作） */
export function getOAuthAccessToken(serverId: string): Promise<OAuthAccessToken> {
  return invoke<OAuthAccessToken>("get_oauth_access_token", { serverId });
}

/** 退出登录：尽力撤销授权并清 token，保留客户端注册 */
export function logoutOAuth(serverId: string): Promise<OAuthLogoutResult> {
  return invoke<OAuthLogoutResult>("logout_oauth", { serverId });
}

export interface OAuthRegistrationClearResult {
  cleared: boolean;
  detail?: string | null;
}

/** 清除客户端注册（DCR ↔ CIMD 切换用），保留服务器 OAuth 配置 */
export function clearOAuthRegistration(
  serverId: string,
): Promise<OAuthRegistrationClearResult> {
  return invoke<OAuthRegistrationClearResult>("clear_oauth_registration", {
    serverId,
  });
}

/** 订阅 OAuth 登录事件 */
export function listenOAuth(
  handler: (payload: OAuthPushPayload) => void,
): Promise<UnlistenFn> {
  return listen<OAuthPushPayload>("mcp://oauth", (event) =>
    handler(event.payload),
  );
}
