import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { OAuthDiagnosticsDialog } from "./OAuthDiagnosticsDialog";

const invokeMock = vi.mocked(invoke);

function renderDialog() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <OAuthDiagnosticsDialog
        serverId="s1"
        serverName="Demo"
        onClose={() => {}}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string) => {
    switch (cmd) {
      case "get_oauth_diagnostics":
        return Promise.resolve({
          configured: true,
          authorized: true,
          refreshable: true,
          client_id: "client-1",
          client_metadata_url: null,
          registration: "dcr",
          token_endpoint_auth_method: null,
          issuer: "https://auth.example",
          authorization_endpoint: "https://auth.example/authorize",
          token_endpoint: "https://auth.example/token",
          registration_endpoint: "https://auth.example/register",
          revocation_endpoint: "https://auth.example/revoke",
          resource: "https://mcp.example/mcp",
          scopes_supported: ["read", "write"],
          granted_scopes: ["read"],
          code_challenge_methods_supported: ["S256"],
          token_endpoint_auth_methods_supported: ["client_secret_post"],
          revocation_endpoint_auth_methods_supported: ["client_secret_post"],
          expires_at: null,
          metadata: { issuer: "https://auth.example" },
        });
      case "get_oauth_access_token":
        return Promise.resolve({
          access_token: "secret-token",
          token_type: "Bearer",
          expires_at: null,
        });
      case "refresh_oauth_token":
        return Promise.resolve({ authorized: true });
      default:
        return Promise.resolve(null);
    }
  }) as typeof invoke);
});

describe("OAuthDiagnosticsDialog", () => {
  it("展示端点/注册方式，且 token 默认掩码", async () => {
    renderDialog();
    expect(await screen.findByText("OAuth 诊断 · Demo")).toBeInTheDocument();
    expect(
      await screen.findByText("https://auth.example/authorize"),
    ).toBeInTheDocument();
    expect(await screen.findByText("dcr")).toBeInTheDocument();
    expect(await screen.findByText("S256")).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "查看 access token" }),
    );
    expect(await screen.findByText(/••••/)).toBeInTheDocument();
    expect(screen.queryByText("secret-token")).not.toBeInTheDocument();
  });
});
