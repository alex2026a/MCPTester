import { describe, expect, it } from "vitest";
import { buildHttpSkeleton } from "./httpSkeleton";
import type { ServerConfig } from "../types/models";

function server(partial: Partial<ServerConfig> = {}): ServerConfig {
  return {
    id: "s1",
    name: "mock",
    transport: "streamable_http",
    stdio: null,
    http: {
      url: "https://user:pass@example.com/mcp?api_key=secret",
      headers: [
        { key: "Authorization", value: "Bearer real-token", secret: false },
        { key: "X-Api-Key", value: "real-api-key", secret: false },
        { key: "X-Trace", value: "trace-1", secret: false },
      ],
      timeout_secs: null,
      allow_insecure_tls: false,
      oauth: null,
    },
    protocol_version_pin: null,
    enabled: true,
    tags: [],
    origin: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    last_connected_at: null,
    ...partial,
  };
}

const request = JSON.stringify({
  jsonrpc: "2.0",
  id: 1,
  method: "tools/call",
  params: { name: "echo", arguments: { token: "body-token", text: "hi" } },
});

describe("buildHttpSkeleton", () => {
  it("只对 Streamable HTTP 生成骨架", () => {
    expect(
      buildHttpSkeleton(server({ transport: "stdio" }), request, "2026-07-28"),
    ).toBeNull();
    expect(
      buildHttpSkeleton(server({ transport: "sse" }), request, "2026-07-28"),
    ).toBeNull();
  });

  it("掩码头、URL query/userinfo、body token", () => {
    const text = buildHttpSkeleton(server(), request, "2026-07-28");
    expect(text).not.toBeNull();
    expect(text).not.toContain("real-token");
    expect(text).not.toContain("real-api-key");
    expect(text).not.toContain("secret");
    expect(text).not.toContain("body-token");
    expect(text).not.toContain("user:pass");
    expect(text).toContain("Authorization: ****");
    expect(text).toContain("X-Api-Key: ****");
    expect(text).toContain("X-Trace: trace-1");
    expect(text).toContain("MCP-Protocol-Version: 2026-07-28");
    expect(text).toContain("Mcp-Session-Id: <session-id>");
  });

  it("OAuth 覆盖显式 Authorization", () => {
    const config = server();
    config.http = {
      ...config.http!,
      oauth: { scopes: ["read"] },
    };
    const text = buildHttpSkeleton(config, request, "2025-11-25");
    expect(text).not.toContain("Bearer real-token");
    expect(text).toContain("Authorization: Bearer ****");
    expect(text).toContain("先 initialize");
    expect(text).not.toContain("MCP-Protocol-Version");
  });
});
