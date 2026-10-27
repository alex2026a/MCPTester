<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="120" alt="MCP Tester" />
</div>

<h1 align="center">MCP Tester</h1>

<p align="center">
  <strong>A desktop client for testing MCP servers</strong><br />
  Connect to any MCP server, inspect its capabilities, invoke tools with generated forms, watch JSON-RPC traffic, and replay past calls.
</p>

<p align="center">
  <a href="README.md">简体中文</a> · <strong>English</strong>
</p>

<p align="center">
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri 2" /></a>
  <a href="https://react.dev/"><img src="https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=white" alt="React 19" /></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white" alt="Rust 2024" /></a>
  <a href="https://modelcontextprotocol.io/"><img src="https://img.shields.io/badge/MCP-2026--07--28-6E56CF" alt="MCP 2026-07-28" /></a>
  <a href="#requirements"><img src="https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows&logoColor=white" alt="Windows 10/11" /></a>
  <a href="../../releases"><img src="https://img.shields.io/badge/version-0.5.0-2F6FEB" alt="version 0.5.0" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-22C55E" alt="MIT License" /></a>
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#usage">Usage</a> ·
  <a href="#authentication-and-oauth">Authentication and OAuth</a> ·
  <a href="#startup-configuration">Startup Configuration</a> ·
  <a href="#data-and-portable-mode">Data and Portable Mode</a> ·
  <a href="#security">Security</a> ·
  <a href="#development">Development</a>
</p>

MCP Tester is a desktop debugging tool for the **Model Context Protocol (MCP)**. It combines server management, capability discovery, schema-driven invocation, JSON-RPC wire inspection, call replay, and regression testing in a single UI. It is designed for local `stdio` servers, remote `Streamable HTTP` servers, and legacy `HTTP + SSE` servers.

> [!NOTE]
> Current version: **v0.5.0**. Windows is the primary release target, and the project is under active development.

## Features

| Area | What you can do |
| --- | --- |
| **Server management** | Add, edit, and delete `stdio`, `Streamable HTTP`, and `HTTP + SSE` servers; reorder them by drag; connect, disconnect, reconnect, and probe; test a connection before saving with a step-by-step handshake, protocol version, and capability counts. |
| **Authentication and OAuth** | No auth, `Authorization` headers, or OAuth 2.1 authorization code + PKCE; dynamic client registration (DCR), CIMD, and pre-registered clients; token refresh, revocation, step-up authorization, and diagnostics. |
| **API explorer** | Enumerate Tools, Resources, Resource Templates, and Prompts; search by name, description, or URI; automatically refresh when the server sends a list-changed notification. |
| **Schema forms** | Generate forms from a tool's `inputSchema`, with required, type, and enum validation; switch to raw JSON editing; explicitly choose "Send anyway" when validation fails; keep unsent drafts per target. |
| **Invocation and cancellation** | Call `tools/call`, `resources/read`, and `prompts/get`; cancel in-flight calls; extract and validate URI template variables; send with `Ctrl+Enter`; refill from recent parameters. |
| **Result viewer** | Render MCP content blocks including text, Markdown, images, audio, resources, links, and structured JSON; rendered / tree / raw views, response / request tabs, and original JSON-RPC frames; copy requests, responses, pairs, Markdown, parameters, text digests, and HTTP request skeletons, with optional redaction. |
| **Traffic inspector** | Pair JSON-RPC requests and responses by id, with duration, size, and status; filter by method, keyword, or errors; pause, follow the latest traffic, and export. View server notifications and stdio `stderr` in separate tabs. |
| **History and replay** | Persist calls in SQLite; filter by server, method, status, keyword, or target; replay a call in the Explorer with one click; copy parameters, delete, clear, or export history as JSON (optionally including raw frames). |
| **Tests and reports** | Manage suites and cases; generate cases from schemas, history, or an LLM; run, cancel, assert, and classify failures; baseline comparison, coverage, and contract drift; export reports as HTML / JSON / JUnit / Markdown. |
| **Import and export** | Recognize Claude Desktop / Cursor `mcpServers` and VS Code `servers` formats; mask secrets when exporting server configuration. |
| **Startup configuration** | Pre-seed servers, a default server, and auto-connect through `mcptester.config.json`; add missing entries without overwriting user changes. |
| **Portable and secure** | Store credentials in the OS keychain for installed builds; keep portable data beside the executable; redact sensitive log fields; spill large results to disk. |

Design goals:

- **Observable**: see the full JSON-RPC round trip, raw frames, notifications, and process output, not just the final result.
- **Reproducible**: replay parameters and results, and keep test cases seeded and baseline-compared, to investigate intermittent failures and verify fixes.
- **Portable**: import and export server configuration between MCP Tester, Claude Desktop, Cursor, and VS Code formats.

## Quick Start

### Download

Download the latest build from [Releases](../../releases).

| Build | Best for | Data and credentials |
| --- | --- | --- |
| **MSI installer** | Daily use, standard install and uninstall | Data in the user data directory; secrets in the OS keychain |
| **Portable ZIP** | No installation, removable media, isolated environments | Data in `data/` next to the executable; secrets and OAuth tokens stored as plaintext in the local database |

> [!WARNING]
> Portable builds store headers, environment variables, OAuth refresh tokens, and client secrets as plaintext in `data/mcptester.db`. Do not share a portable folder containing real credentials. The MSI is currently unsigned, so Windows SmartScreen may warn on first launch.

### Requirements

| Requirement | Details |
| --- | --- |
| Operating system | Windows 10 / 11 (x64) |
| WebView2 | Built into Windows 11; Windows 10 may require the WebView2 Runtime |
| Node.js | 20 or newer |
| pnpm | 9 or newer (the repository uses `pnpm-lock.yaml`) |
| Rust | Stable, with edition 2024 support |
| Build tools | Visual Studio Build Tools with "Desktop development with C++" |

### Run from source

```bash
pnpm install
pnpm tauri dev
```

The first run downloads and compiles Rust dependencies, which can take a while depending on your network and machine.

### Build and package

```bash
pnpm build        # TypeScript type-check + frontend build
pnpm tauri build  # build the MSI
```

The MSI is written to `src-tauri/target/release/bundle/msi/`. The repository also ships a build script with archiving and verification:

| Command | Profile | Output |
| --- | --- | --- |
| `pnpm release` | Speed-first (thin LTO, `codegen-units=16`, `opt-level=3`) | `artifacts/<version>/` |
| `pnpm release:size` | Size-first (fat LTO, `codegen-units=1`, `opt-level=s`) | `artifacts-size/<version>/` |
| `pnpm portable` / `pnpm msi` | Build only the selected form | `artifacts/<version>/` |

The size-first profile requires a full rebuild and takes noticeably longer; it is intended for distribution builds. Use the default profile for day-to-day development.

To build a portable copy manually:

```powershell
pnpm tauri build --no-bundle
# Copy src-tauri/target/release/mcptester.exe to a target folder
# Create an empty mcptester.portable file next to it to enable portable mode
```

## Usage

### 1. Add and connect a server

- Open **Servers** and click **New server**.
- `stdio`: provide the command, one argument per line, environment variables, and working directory. Keep the command resolution mode on `resolve`; `cmd_shell` has escaping risks; `never` is mainly for troubleshooting.
- `HTTP / SSE`: provide the MCP endpoint URL, headers, and an optional timeout override. Enable "Allow insecure TLS" only for local self-signed certificates.
- Choose one of three authentication modes: **None**, **Authorization header** (a full `Bearer xxx` value or a bare token), or **OAuth 2.1** (`Streamable HTTP` only; see [Authentication and OAuth](#authentication-and-oauth)).
- **Test connection** validates the configuration without saving it or writing history. After saving, click **Connect**, then open **Explorer** to discover capabilities.
- With several servers, drag the handle to reorder them; server pickers in Explorer, History, and Tests group by connection state and keep this order within each group.

### 2. Explore and invoke

- Open **Explorer**, select a connected server, and refresh capabilities if needed.
- Tabs cover Tools, Resources, Resource Templates, and Prompts; the left-hand list supports keyword search.
- Tool forms are generated from the JSON Schema; switch to JSON mode to edit arguments directly.
- Missing required fields or type mismatches block sending by default. Choose **Send anyway** to bypass client-side validation; this option is not offered when JSON parsing fails.
- **Recent parameters** lists past calls for the same server and target, and refills them with one click and an undo action. Unsent drafts are kept per target for the current session.
- Press `Ctrl+Enter` to send. Resource templates extract `{variables}` and require all values before reading. In-flight calls can be cancelled.

### 3. Inspect results

- The result header shows status, duration, protocol version, transport, timestamp, and size.
- **Rendered** displays MCP content blocks; **Tree** shows the JSON structure; **Raw** preserves the original response.
- Switch between **Response** and **Request**. Requests and responses prefer the original JSON-RPC frames captured on the wire; when frames are missing or too large, they are reconstructed from the call record and marked as "raw / partial / reconstructed".
- The copy menu offers request, response, request+response (Markdown), request+response (JSON), request parameters, response (JSON-RPC), HTTP request skeleton, and text digest. Enable **redacted copy** to mask sensitive values; the UI warns about sensitive parameter fields before copying.
- The HTTP request skeleton is available for `Streamable HTTP` only and intentionally omits session details such as `Mcp-Session-Id`; URLs, headers, and bodies are redacted.
- Results larger than 1 MB are truncated in the database. The full payload is written to `data/results/<record-id>.json` and can be opened from the result view; reading more than 10 MB of full content asks for confirmation first.

### 4. Inspect traffic

- Expand the **Event panel** at the bottom of the Explorer.
- The **Wire** tab pairs requests and responses by JSON-RPC id and shows direction, duration, size, and status. Filter by method, keyword, or errors.
- Pause, follow the latest traffic, clear, or export the wire log. Server notifications and stdio `stderr` have their own tabs.
- When the server sends a `*/list_changed` notification, the capability list refreshes automatically.

### 5. History and replay

- **History** records tool calls, resource reads, prompt requests, and probes.
- Filter by server, method, status, keyword, or target, then inspect the full parameters and result. Filters are mirrored in the URL so other pages can link to a specific view.
- **Load in Explorer** returns to the Explorer, selects the server and target, and fills in the original arguments. Parameters can also be copied for manual use.
- Delete individual records, clear the current filter, or export history as JSON. Exports can optionally include raw frames (significantly larger, capped at 50 MB).
- Retention days and maximum row count are configurable in Settings.

### 6. Tests and reports

- **Tests** manages suites and cases per server; a suite must be bound to one MCP server. Cases support `tools/call`, `resources/read`, and `prompts/get`, with priority, risk level, and an enabled flag.
- **Generate cases** offers three profiles: **smoke**, **standard regression**, and **thorough**. Choose whether to include boundary values, negative cases, resources, prompts, and historical parameters; destructive tools are skipped by default. Generated cases are previewed, selected, and saved with a fixed seed for reproducibility.
- **LLM assist** exports only interface definitions (no credentials or history parameters). Send the prompt to any model, then import the returned JSON and review the cases manually.
- Runs support per-case timeouts and retries for environment errors, and can execute a whole suite or selected cases with progress and cancellation.
- **Reports** summarizes pass/fail/skip counts, duration, failure classification, and coverage. You can set a baseline, compare runs, and rerun failed cases.
- **Contract drift** compares the current capability snapshot with the suite baseline and lists added, removed, or changed capabilities.
- Reports can be exported as HTML, JSON, JUnit XML, or Markdown, optionally including sensitive data.

### 7. Import and export

- Import **Claude Desktop / Cursor** `{ "mcpServers": { ... } }` and **VS Code** `{ "servers": { ... } }` configurations.
- Import from a file or paste JSON directly.
- Sensitive values are detected by key name first, then by value pattern. Installed builds store them in the OS keychain; portable builds keep them in the data directory.
- Export in `claude` or `vscode` format. All sensitive server configuration values are replaced with `${MCPTESTER_SECRET}`.

## Authentication and OAuth

The server form offers three authentication modes:

| Mode | Best for | Notes |
| --- | --- | --- |
| **None** | Local or intranet servers | No `Authorization` header is sent |
| **Authorization header** | Existing Bearer tokens or API keys | Full `Bearer xxx` or a bare token |
| **OAuth 2.1** | Remote servers following the MCP authorization spec | Authorization code + PKCE, `Streamable HTTP` only |

OAuth login flow:

1. Select OAuth 2.1, enter scopes (leave empty to let the authorization server metadata choose), and optionally fill in `client_id`, `client_secret`, and the redirect URI.
2. After saving, click **Login** on the server card. The app starts a loopback callback listener (default `http://127.0.0.1:33318/oauth/callback`) and opens the system browser.
3. When authorization completes, tokens are stored automatically. A 401 prompts you to log in; a 403 `insufficient_scope` shows the missing scope and a **Grant more scopes** action.
4. The card menu offers **Diagnostics**, **Log out**, and **Clear registration**. Diagnostics shows endpoints, registration mode, PKCE, scopes, and expiry, and lets you force a token refresh or temporarily reveal the access token.

Interoperability options (advanced):

- If metadata discovery fails, enter the authorization / token / registration / issuer / revocation endpoints manually. Non-compliant servers can tolerate a missing issuer.
- Client registration supports **DCR** (default), **CIMD** (`client_metadata_url`), and a pre-registered `client_id`. To switch registration modes after logging in with DCR/CIMD, clear the registration first.
- You can pin the token endpoint auth method (`client_secret_basic`, `client_secret_post`, or `none`).
- Extra authorization request parameters can be added one `key=value` per line; reserved parameters such as `state`, PKCE, and `client_id` cannot be overridden.
- Custom headers are sent only to the same origin as the MCP server by default. Forwarding them to a cross-origin authorization server must be enabled explicitly; confirm this will not leak gateway secrets.
- OAuth mode ignores the `Authorization` header. Authorizations without a refresh token last for the current session only and require logging in again after a restart.

## Startup Configuration

You can pre-seed servers at startup. See [`mcptester.config.example.json`](mcptester.config.example.json) for a complete example:

```json
{
  "mcptester": {
    "version": 1,
    "autoConnect": true,
    "defaultServer": "local-dev",
    "secretHeaders": ["X-Custom-Token"],
    "secretEnv": []
  },
  "servers": {
    "local-dev": {
      "name": "Local dev",
      "type": "http",
      "url": "http://127.0.0.1:8080/mcp",
      "headers": {
        "Authorization": "Bearer ${MCPTESTER_SECRET}"
      },
      "timeout_secs": 30
    },
    "filesystem": {
      "name": "Local filesystem",
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "D:\\data"],
      "env": {},
      "shell_wrap": "resolve"
    }
  }
}
```

**Discovery order**

1. Command-line argument `--config <path>`
2. Environment variable `MCPTESTER_CONFIG`
3. `mcptester.config.json` next to the executable
4. `mcptester.config.json` in the data directory
5. `mcptester.config.json` in the Tauri resource directory

**Application rules**

- Only missing servers are added. Existing configuration is **never overwritten**.
- A deleted pre-seeded server does not come back automatically. Use **Reapply configuration** in Settings to restore missing entries.
- `autoConnect` and `defaultServer` are applied only on the first seed or when configuration is manually reapplied.
- `secretHeaders` and `secretEnv` mark additional keys as sensitive. A value of `${MCPTESTER_SECRET}` means "to be configured" and is not written to the database or keychain.
- Servers seeded from the configuration file keep their headers and environment variables as plaintext in local SQLite; exports still mask them.
- OAuth configuration is not seeded from the startup file. Log in and save from the server editor instead; existing OAuth servers can still participate in `autoConnect`.

> [!IMPORTANT]
> Do not commit a `mcptester.config.json` containing real credentials. The repository's `.gitignore` ignores this file and keeps only the example.

## Data and Portable Mode

| Mode | How it is enabled | Data directory | Credential storage |
| --- | --- | --- | --- |
| **Installed** | Default | `%APPDATA%\com.mcptester.desktop` | OS keychain (including OAuth tokens) |
| **Portable** | `mcptester.portable` next to the executable | `<program directory>\data` | Plaintext in the local database (including OAuth tokens) |
| **Custom** | Set `MCPTESTER_DATA_DIR` | The specified directory (relative paths resolve against the program directory) | Determined by the portable marker |

If the portable directory is not writable, the app falls back to the user data directory and reports in Settings that portable credential handling is disabled for this run.

Common files:

| Path | Contents |
| --- | --- |
| `mcptester.db` | Server configuration, OAuth credentials, settings, call history, and test reports |
| `results/` | Full results for payloads larger than 1 MB |
| `logs/` | Daily rolling application logs |
| `mcptester.config.json` | Optional startup configuration |

Logs for installed builds default to `%LOCALAPPDATA%\com.mcptester.desktop\logs`; the exact paths are shown in Settings.

## Security

- **Credential isolation**: In installed mode, sensitive headers, environment variables, OAuth refresh tokens, and client secrets are stored in the OS credential store (Windows Credential Manager / macOS Keychain / Linux Secret Service). SQLite stores only reference indexes.
- **OAuth boundaries**: Access tokens are not persisted. Logging out clears local tokens and attempts remote revocation when a revocation endpoint is configured. A `403 insufficient_scope` requests only the missing scope.
- **Masked editing**: The UI displays masked values such as `Beare****5678`. Leaving the field blank keeps the existing secret; entering a new value replaces it, preventing masks from being written back to the credential store.
- **Masked exports**: Exporting server configuration replaces sensitive values with `${MCPTESTER_SECRET}`. History exports preserve the original requests and responses; report exports exclude sensitive data by default. Review both before sharing.
- **Redacted copy**: The copy menu offers redacted copy. Parameters, JSON-RPC frames, HTTP request skeletons, and text digests mask Bearer tokens, JWTs, `sk-` keys, URL token parameters, and URL userinfo.
- **Log redaction**: Sensitive fields are matched by exact name, and value patterns such as Bearer tokens, JWTs, `sk-` keys, URL token parameters, and URL userinfo are redacted. `stderr` and notifications stay in memory buffers and are not written to log files.
- **Local-first**: The app contains no telemetry or cloud sync. Server configuration, call history, test reports, and response content stay on the local machine.
- **Explicit risk switches**: "Allow insecure TLS" is intended only for local self-signed certificates and is off by default. Cross-origin OAuth header forwarding is off by default. Portable-mode plaintext credentials are clearly flagged in the UI.

## Protocol Support

| Generation | Protocol versions | Negotiation |
| --- | --- | --- |
| **Modern** | `2026-07-28` | `server/discover` |
| **Legacy** | `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` | `initialize` handshake |

Automatic negotiation is the default: try the modern generation first, then fall back to the latest legacy version. You can pin a server to `2026-07-28` or `2025-11-25`.

Supported interactions include:

- `tools/list`, `tools/call`
- `resources/list`, `resources/read`, `resources/templates/list`
- `prompts/list`, `prompts/get`
- `subscriptions/listen` for modern list-changed notifications
- Legacy `ping` health checks; modern generation uses an idempotent capability listing as a probe
- Call cancellation and in-flight request aborts
- OAuth 2.1 authorization code + PKCE, including RFC 9728 resource metadata, RFC 8414 authorization server metadata, DCR, and CIMD client registration

> [!NOTE]
> `HTTP + SSE` is a deprecated MCP transport kept for compatibility with older servers. New servers should use `Streamable HTTP`.

## Architecture

```mermaid
flowchart LR
    subgraph FE["Frontend · React 19 + TypeScript"]
        UI["Servers / Explorer / History / Tests / Reports / Settings"]
        STATE["TanStack Query + Zustand"]
        UI <--> STATE
    end

    subgraph BE["Backend · Rust / Tauri 2"]
        CMD["Tauri Commands"]
        MGR["Connection Manager"]
        OAUTH["OAuth 2.1 Service"]
        TEST["Test Runner"]
        WIRE["JSON-RPC Wire Log"]
        DB[("SQLite")]
        KEY[("OS Keychain")]
        LOG["Rolling Logs"]
    end

    FE -->|Tauri IPC| CMD
    CMD --> MGR
    CMD --> OAUTH
    CMD --> TEST
    MGR --> WIRE
    TEST --> MGR
    OAUTH --> KEY
    CMD --> DB
    CMD --> LOG

    MGR --> STDIO["stdio"]
    MGR --> HTTP["Streamable HTTP"]
    MGR --> SSE["HTTP + SSE"]
    STDIO --> SERVER["MCP Server"]
    HTTP --> SERVER
    SSE --> SERVER
```

| Layer | Technology |
| --- | --- |
| Desktop shell | Tauri 2 |
| Frontend | React 19, TypeScript, Vite, Tailwind CSS |
| Routing and state | React Router, TanStack Query, Zustand |
| Backend | Rust 2024, Tokio, rmcp 3.5 |
| Authentication | OAuth 2.1 (oauth2 5.0), reqwest 0.13 |
| Credentials | keyring-core with native platform stores (including OAuth tokens) |
| Storage | SQLite (rusqlite, bundled) |
| Logging | tracing + tracing-appender (daily rolling) |
| Testing | Vitest, Testing Library, cargo test, built-in mock MCP server, OAuth flow integration tests |

## Development

### Quality checks

```bash
pnpm lint
pnpm test

cd src-tauri
cargo test --features mock-server
cargo clippy --all-targets
```

### Built-in mock server

The repository includes a deterministic mock MCP server covering tool calls, resources, resource templates, prompts, list-changed notifications, both protocol generations, and OAuth login.

```bash
cd src-tauri
cargo run --features mock-server --bin mock-mcp-server -- --transport http --port 18080
```

Then add a server in the app with the URL `http://127.0.0.1:18080/mcp`.

Useful flags:

- `--transport stdio|http|sse`
- `--era both|modern|legacy`
- `--port 18080`
- `--fastmcp-compat --token mock-token`: emulate FastMCP / Python server authentication and session behavior
- `--oauth`: enable the OAuth 2.1 mock; combine with `--oauth-no-refresh`, `--oauth-dcr-secret`, `--oauth-token-ttl <seconds>`, `--oauth-cimd`, and `--oauth-require-authorize-param key=value` to vary registration, token lifetime, and authorization parameter checks

### Probe CLI

You can connect and invoke without opening the desktop UI. The probe is excluded from release builds by default and requires the `dev-tools` feature:

```bash
cd src-tauri
cargo run --features dev-tools --bin probe -- \
  --http http://127.0.0.1:18080/mcp \
  --tool echo \
  --args '{"text":"hello"}'
```

Use `--stdio` to debug local process-based servers.

## Project Structure

```text
.
├─ src/                         # React frontend
│  ├─ pages/                    # Servers / Explorer / History / Tests / Reports / Settings
│  ├─ components/               # Schema forms, result and wire viewers, OAuth dialogs, event panel
│  ├─ api/                      # Tauri IPC wrappers
│  ├─ stores/                   # Replay, drafts, and toast state
│  └─ i18n/                     # Chinese copy
├─ src-tauri/
│  ├─ src/mcp/                  # Connections, transports, models, wire log
│  │  └─ oauth/                 # OAuth 2.1 login, client identity, credential storage
│  ├─ src/commands/             # Tauri commands
│  ├─ src/storage/              # SQLite repositories and migrations
│  ├─ src/testing/              # Test generation, assertions, coverage, contracts, reports
│  ├─ src/bootstrap/            # Path resolution and startup configuration
│  ├─ src/bin/                  # mock-mcp-server / probe
│  └─ tests/                    # MCP client and OAuth integration tests
├─ LICENSE
├─ mcptester.config.example.json
├─ package.json
├─ README.md
└─ README.en.md
```

## Known Limitations

- Only the Windows MSI is configured and released; macOS and Linux code paths are not packaged or verified yet.
- The UI is currently Chinese-only; the `i18n` directory is structured for additional languages.
- Automatic reconnect after a disconnect is still under development; use the **Reconnect** button at the top of the page for now.
- OAuth supports authorization code + PKCE over Streamable HTTP only. Device code and client credentials flows are not implemented, and legacy `HTTP + SSE` is not supported for OAuth.
- Results larger than 1 MB are truncated to the first 1 MB in the database; open `results/` for the full payload.
- The legacy `HTTP + SSE` transport is compatibility-only; some newer servers may no longer support it.

## Roadmap

- [ ] macOS / Linux packaging and signing
- [ ] Automatic reconnect and connection keep-alive
- [ ] English UI
- [ ] OAuth device code and enterprise SSO scenarios
- [ ] CI and automated releases

## License

Released under the [MIT License](LICENSE).

Copyright © 2026 alex2026a
