import { useState, type ReactNode } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  getOAuthAccessToken,
  getOAuthDiagnostics,
  refreshOAuthToken,
  type OAuthAccessToken,
  type OAuthDiagnostics,
} from "../api/oauth";
import { JsonViewer } from "./JsonViewer";
import { toast } from "../stores/toast";

interface OAuthDiagnosticsDialogProps {
  serverId: string;
  serverName: string;
  onClose: () => void;
}

function valueOrDash(value: string | null | undefined): string {
  return value && value.length > 0 ? value : "—";
}

function listOrDash(values: string[] | undefined): string {
  return values && values.length > 0 ? values.join(" ") : "—";
}

function summary(diagnostics: OAuthDiagnostics) {
  return {
    registration: diagnostics.registration,
    client_id: diagnostics.client_id,
    client_metadata_url: diagnostics.client_metadata_url,
    issuer: diagnostics.issuer,
    authorization_endpoint: diagnostics.authorization_endpoint,
    token_endpoint: diagnostics.token_endpoint,
    registration_endpoint: diagnostics.registration_endpoint,
    revocation_endpoint: diagnostics.revocation_endpoint,
    resource: diagnostics.resource,
    scopes_supported: diagnostics.scopes_supported,
    granted_scopes: diagnostics.granted_scopes,
    code_challenge_methods_supported:
      diagnostics.code_challenge_methods_supported,
    token_endpoint_auth_methods_supported:
      diagnostics.token_endpoint_auth_methods_supported,
    revocation_endpoint_auth_methods_supported:
      diagnostics.revocation_endpoint_auth_methods_supported,
    expires_at: diagnostics.expires_at,
  };
}

export function OAuthDiagnosticsDialog({
  serverId,
  serverName,
  onClose,
}: OAuthDiagnosticsDialogProps) {
  const diagnostics = useQuery({
    queryKey: ["oauthDiagnostics", serverId],
    queryFn: () => getOAuthDiagnostics(serverId),
  });
  const refresh = useMutation({
    mutationFn: () => refreshOAuthToken(serverId),
    onSuccess: () => {
      toast.success("OAuth token 已强制刷新");
      void diagnostics.refetch();
    },
    onError: (error) => toast.fromError(error, "强制刷新失败"),
  });
  const [token, setToken] = useState<OAuthAccessToken | null>(null);
  const [revealed, setRevealed] = useState(false);
  const revealToken = useMutation({
    mutationFn: () => getOAuthAccessToken(serverId),
    onSuccess: (value) => {
      setToken(value);
      setRevealed(false);
    },
    onError: (error) => toast.fromError(error, "读取 access token 失败"),
  });

  async function copyJson() {
    if (!diagnostics.data) return;
    try {
      await navigator.clipboard.writeText(
        JSON.stringify(summary(diagnostics.data), null, 2),
      );
      toast.success("已复制诊断摘要");
    } catch {
      toast.error("复制失败");
    }
  }

  async function copyToken() {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token.access_token);
      toast.success("已复制 access token");
    } catch {
      toast.error("复制失败");
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <div className="max-h-[90vh] w-full max-w-4xl space-y-3 overflow-auto rounded-lg bg-white p-5 shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-slate-800">
              OAuth 诊断 · {serverName}
            </h3>
            <p className="mt-1 text-xs text-slate-500">
              显式读取授权服务器元数据；不包含 client_secret / token。
            </p>
          </div>
          <button
            type="button"
            className="text-sm text-slate-400 hover:text-slate-600"
            onClick={onClose}
            aria-label="关闭诊断"
          >
            ×
          </button>
        </div>

        {diagnostics.isPending ? (
          <p className="text-sm text-slate-500">正在读取元数据…</p>
        ) : null}

        {diagnostics.isError ? (
          <p className="rounded border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-600">
            {String(diagnostics.error)}
          </p>
        ) : null}

        {diagnostics.data ? (
          <>
            {diagnostics.data.discovery_error ? (
              <p className="rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700">
                元数据发现失败：{diagnostics.data.discovery_error}
              </p>
            ) : null}
            <div className="grid grid-cols-1 gap-x-6 gap-y-1.5 text-xs md:grid-cols-2">
              <Row label="状态">
                {diagnostics.data.authorized ? "已授权" : "未授权"}
                {diagnostics.data.refreshable ? " · 可刷新" : " · 无 refresh"}
              </Row>
              <Row label="注册方式">
                {valueOrDash(diagnostics.data.registration)}
              </Row>
              <Row label="client_id">
                {valueOrDash(diagnostics.data.client_id)}
              </Row>
              <Row label="client_metadata_url">
                {valueOrDash(diagnostics.data.client_metadata_url)}
              </Row>
              <Row label="issuer">{valueOrDash(diagnostics.data.issuer)}</Row>
              <Row label="resource">{diagnostics.data.resource}</Row>
              <Row label="authorization_endpoint">
                {valueOrDash(diagnostics.data.authorization_endpoint)}
              </Row>
              <Row label="token_endpoint">
                {valueOrDash(diagnostics.data.token_endpoint)}
              </Row>
              <Row label="registration_endpoint">
                {valueOrDash(diagnostics.data.registration_endpoint)}
              </Row>
              <Row label="revocation_endpoint">
                {valueOrDash(diagnostics.data.revocation_endpoint)}
              </Row>
              <Row label="PKCE">
                {listOrDash(diagnostics.data.code_challenge_methods_supported)}
              </Row>
              <Row label="token auth">
                {listOrDash(
                  diagnostics.data.token_endpoint_auth_methods_supported,
                )}
              </Row>
              <Row label="revocation auth">
                {listOrDash(
                  diagnostics.data.revocation_endpoint_auth_methods_supported,
                )}
              </Row>
              <Row label="scopes_supported">
                {listOrDash(diagnostics.data.scopes_supported)}
              </Row>
              <Row label="granted scopes">
                {listOrDash(diagnostics.data.granted_scopes)}
              </Row>
              <Row label="过期时间">
                {valueOrDash(diagnostics.data.expires_at)}
              </Row>
            </div>

            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50 disabled:opacity-50"
                onClick={() => refresh.mutate()}
                disabled={refresh.isPending}
              >
                {refresh.isPending ? "刷新中…" : "强制刷新 token"}
              </button>
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50 disabled:opacity-50"
                onClick={() => revealToken.mutate()}
                disabled={revealToken.isPending}
              >
                {revealToken.isPending ? "读取中…" : "查看 access token"}
              </button>
              <button
                type="button"
                className="rounded border border-slate-200 px-3 py-1.5 text-xs hover:bg-slate-50"
                onClick={() => void copyJson()}
              >
                复制诊断摘要
              </button>
            </div>

            {token ? (
              <div className="space-y-2 rounded border border-amber-200 bg-amber-50 p-3">
                <p className="text-xs font-medium text-amber-800">
                  access token（{token.token_type}
                  {token.expires_at ? `，过期 ${token.expires_at}` : ""}）
                </p>
                <div className="break-all rounded bg-white/80 p-2 font-mono text-[11px] text-slate-700">
                  {revealed ? token.access_token : "••••••••（点击显示）"}
                </div>
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    className="rounded border border-amber-300 bg-white px-2 py-1 text-xs text-amber-800 hover:bg-amber-100"
                    onClick={() => setRevealed((value) => !value)}
                  >
                    {revealed ? "隐藏" : "显示"}
                  </button>
                  <button
                    type="button"
                    className="rounded border border-amber-300 bg-white px-2 py-1 text-xs text-amber-800 hover:bg-amber-100"
                    onClick={() => void copyToken()}
                  >
                    复制
                  </button>
                </div>
                <p className="text-[11px] text-amber-700">
                  token 仅用于本地对照测试，不要粘贴到公开渠道。
                </p>
              </div>
            ) : null}

            {diagnostics.data.metadata ? (
              <div className="space-y-1">
                <p className="text-xs font-medium text-slate-600">
                  授权服务器元数据（原始 JSON）
                </p>
                <JsonViewer
                  value={diagnostics.data.metadata}
                  defaultMode="tree"
                  contentClassName="max-h-72 overflow-auto"
                />
              </div>
            ) : null}
          </>
        ) : null}
      </div>
    </div>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="flex gap-2">
      <span className="w-44 shrink-0 text-slate-400">{label}</span>
      <span className="break-all text-slate-700">{children}</span>
    </div>
  );
}
