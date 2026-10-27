//! 服务器配置导入/导出（04 文档第 9 节）
//!
//! - 导入：识别 Claude Desktop / Cursor（`mcpServers`）与 VS Code（`servers` + `type`）两种格式；
//!   敏感项按「键名优先 + 值模式兜底」判定，命中者入 keyring；
//! - 导出：`claude`（mcpServers）与 `vscode`（servers + type）两种格式；
//!   敏感值一律输出占位符 `${MCPTESTER_SECRET}`，不导出明文。

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::State;

use crate::commands::AppState;
use crate::commands::server::{
    SecretAction, SecretInput, SecretKindInput, ServerDraft, add_server_inner,
};
use crate::mcp::models::{HttpConfig, KeyValue, ServerConfig, ShellWrap, TransportKind};
use crate::storage::server_repo;

pub(crate) const SECRET_PLACEHOLDER: &str = "${MCPTESTER_SECRET}";

#[derive(Debug, Clone, Serialize)]
pub struct ImportIssue {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub added: u32,
    pub updated: u32,
    pub skipped: Vec<ImportIssue>,
    pub failed: Vec<ImportIssue>,
}

/// 导入服务器配置（json 文本；自动识别格式）
#[tauri::command]
pub async fn import_servers(
    state: State<'_, AppState>,
    json: String,
) -> Result<ImportReport, String> {
    import_servers_inner(&state, &json).await
}

/// 从文件导入服务器配置（v0.1.1：原生文件选择）
#[tauri::command]
pub async fn import_servers_from_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportReport, String> {
    let json =
        std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败「{path}」：{e}"))?;
    import_servers_inner(&state, &json).await
}

async fn import_servers_inner(state: &AppState, json: &str) -> Result<ImportReport, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let entries = detect_entries(&root)
        .ok_or_else(|| "无法识别格式（需为 mcpServers 或 servers 结构）".to_string())?;

    let mut report = ImportReport {
        added: 0,
        updated: 0,
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for (name, value) in entries {
        match build_draft(&name, &value) {
            Ok(draft) => match add_server_inner(state, draft).await {
                Ok(_) => report.added += 1,
                Err(e) => report.failed.push(ImportIssue { name, reason: e }),
            },
            Err(reason) => report.skipped.push(ImportIssue { name, reason }),
        }
    }
    Ok(report)
}

/// 导出预览（v0.1.1：返回 JSON 文本，不落盘）
#[tauri::command]
pub async fn preview_export_servers(
    state: State<'_, AppState>,
    format: String,
) -> Result<String, String> {
    let servers = state
        .db
        .with_read_blocking(server_repo::list)
        .map_err(|e| e.to_string())?;
    build_export(&servers, &format)
}

/// 单服务器配置 JSON（Claude 格式；敏感值为占位符）
#[tauri::command]
pub async fn get_server_json(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let servers = state
        .db
        .with_read_blocking(server_repo::list)
        .map_err(|e| e.to_string())?;
    let server = servers
        .into_iter()
        .find(|server| server.id == id)
        .ok_or_else(|| "服务器不存在".to_string())?;
    build_export(&[server], "claude")
}

/// 导出服务器配置（返回文件路径）
#[tauri::command]
pub async fn export_servers(
    state: State<'_, AppState>,
    format: String,
    path: Option<String>,
) -> Result<String, String> {
    let servers = state
        .db
        .with_read_blocking(server_repo::list)
        .map_err(|e| e.to_string())?;
    let payload = build_export(&servers, &format)?;
    let target = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => state
            .data_dir
            .join(format!("mcptester-export-{format}.json")),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&target, payload).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

// ---------------------------------------------------------------------------
// 解析（纯函数，便于单测）
// ---------------------------------------------------------------------------

/// 敏感键提示（配置文件 `mcptester.secretHeaders` / `secretEnv`）
#[derive(Debug, Clone, Default)]
pub(crate) struct SecretHints {
    pub headers: Vec<String>,
    pub env: Vec<String>,
}

/// 识别格式并返回条目列表
pub(crate) fn detect_entries(root: &Value) -> Option<Vec<(String, Value)>> {
    if let Some(map) = root.get("mcpServers").and_then(Value::as_object) {
        return Some(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }
    if let Some(map) = root.get("servers").and_then(Value::as_object) {
        return Some(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }
    None
}

/// 单条目 → ServerDraft（导入模式：敏感值剥离到 secrets）
fn build_draft(name: &str, value: &Value) -> Result<ServerDraft, String> {
    build_draft_with_options(name, value, false, &SecretHints::default())
}

/// 单条目 → ServerDraft
///
/// `keep_secret_values = true`：敏感值保留在 `KeyValue.value`（供配置文件 seed
/// 明文入库），不生成 `SecretInput`；否则沿用导入行为（值剥离到 secrets，后续写凭据库）。
pub(crate) fn build_draft_with_options(
    name: &str,
    value: &Value,
    keep_secret_values: bool,
    hints: &SecretHints,
) -> Result<ServerDraft, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "条目不是对象".to_string())?;

    // 传输类型：type 字段优先；有 command → stdio；有 url → http（无 type 时按 sse 兼容旧格式）
    let described_type = object.get("type").and_then(Value::as_str);
    let has_command = object.get("command").is_some();
    let has_url = object.get("url").is_some();
    let transport = match described_type {
        Some("stdio") => TransportKind::Stdio,
        Some("http") | Some("streamable_http") | Some("streamableHttp") => {
            TransportKind::StreamableHttp
        }
        Some("sse") => TransportKind::Sse,
        Some(other) => return Err(format!("不支持的传输类型：{other}")),
        None if has_command => TransportKind::Stdio,
        None if has_url => TransportKind::Sse, // 未标 type 的 URL 条目按旧协议处理
        None => return Err("缺少 command 或 url".to_string()),
    };

    let display_name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(name)
        .to_string();
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let tags = object
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_else(|| vec!["imported".to_string()]);
    let protocol_version_pin = object
        .get("protocol_version_pin")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let mut secrets: Vec<SecretInput> = Vec::new();

    let stdio = if transport == TransportKind::Stdio {
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| "缺少 command".to_string())?
            .to_string();
        let args = object
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let env = collect_key_values(
            object.get("env"),
            SecretKindInput::Env,
            &mut secrets,
            keep_secret_values,
            &hints.env,
        );
        let shell_wrap = match object.get("shell_wrap").and_then(Value::as_str) {
            Some("cmd_shell") => ShellWrap::CmdShell,
            Some("never") => ShellWrap::Never,
            _ => ShellWrap::Resolve,
        };
        Some(crate::mcp::models::StdioConfig {
            command,
            args,
            env,
            cwd: object
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            shell_wrap,
        })
    } else {
        None
    };

    let http = if transport == TransportKind::Stdio {
        None
    } else {
        let url = object
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| "缺少 url".to_string())?
            .to_string();
        let headers = collect_key_values(
            object.get("headers"),
            SecretKindInput::Header,
            &mut secrets,
            keep_secret_values,
            &hints.headers,
        );
        Some(HttpConfig {
            url,
            headers,
            timeout_secs: object.get("timeout_secs").and_then(Value::as_u64),
            allow_insecure_tls: object
                .get("allow_insecure_tls")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            oauth: None,
        })
    };

    Ok(ServerDraft {
        name: display_name,
        transport,
        stdio,
        http,
        protocol_version_pin,
        enabled,
        tags,
        secrets,
    })
}

/// 键值集合收集：键名优先 + 值模式兜底的敏感判定
fn collect_key_values(
    value: Option<&Value>,
    kind: SecretKindInput,
    secrets: &mut Vec<SecretInput>,
    keep_secret_values: bool,
    hint_keys: &[String],
) -> Vec<KeyValue> {
    let Some(map) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for (key, raw) in map {
        let Some(text) = raw.as_str() else { continue };
        // 占位符：标记"待配置"（不写入明文，也不写凭据库）
        if text == SECRET_PLACEHOLDER {
            items.push(KeyValue {
                key: key.clone(),
                value: None,
                secret: true,
            });
            continue;
        }
        let sensitive = hint_keys.iter().any(|hint| hint == key)
            || is_sensitive_key(key)
            || looks_like_secret_value(text);
        if sensitive {
            if keep_secret_values {
                // 配置文件 seed：保留明文，仅标记敏感（导出时仍会脱敏）
                items.push(KeyValue {
                    key: key.clone(),
                    value: Some(text.to_string()),
                    secret: true,
                });
            } else {
                secrets.push(SecretInput {
                    kind: kind.clone(),
                    key: key.clone(),
                    action: SecretAction::Replace,
                    value: Some(text.to_string()),
                });
                items.push(KeyValue {
                    key: key.clone(),
                    value: None, // 明文不入库（由 add 流程写入 keyring）
                    secret: true,
                });
            }
        } else {
            items.push(KeyValue {
                key: key.clone(),
                value: Some(text.to_string()),
                secret: false,
            });
        }
    }
    items
}

/// 敏感键名判定（键名优先：authorization / cookie / *-key / *-token / *-secret 等）
fn is_sensitive_key(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    let normalized = lowered.strip_prefix("x-").unwrap_or(&lowered);
    const EXACT: &[&str] = &[
        "authorization",
        "cookie",
        "token",
        "access_token",
        "api-key",
        "apikey",
        "key",
        "secret",
        "password",
        "credential",
    ];
    const SUFFIX: &[&str] = &[
        "-key",
        "-token",
        "-secret",
        "-password",
        "-credential",
        "_pat", // GitHub PAT 等个人访问令牌命名
        "_token",
        "_secret",
        "_key",
    ];
    EXACT.contains(&normalized) || SUFFIX.iter().any(|suffix| normalized.ends_with(suffix))
}

/// 值模式兜底（Bearer / sk- / JWT）
fn looks_like_secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("Bearer ")
        || trimmed.starts_with("sk-")
        || (trimmed.starts_with("eyJ") && trimmed.matches('.').count() >= 2)
}

// ---------------------------------------------------------------------------
// 导出（纯函数，便于单测）
// ---------------------------------------------------------------------------

fn build_export(servers: &[ServerConfig], format: &str) -> Result<String, String> {
    let mut map = Map::new();
    for server in servers {
        map.insert(server.name.clone(), export_entry(server));
    }
    let body = match format {
        "claude" => serde_json::json!({ "mcpServers": map }),
        "vscode" => serde_json::json!({ "servers": map }),
        other => return Err(format!("不支持的导出格式：{other}")),
    };
    let mut root = body;
    if let Some(object) = root.as_object_mut() {
        object.insert(
            "_exported_by".to_string(),
            Value::String(format!("MCP Tester v{}", env!("CARGO_PKG_VERSION"))),
        );
    }
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn export_entry(server: &ServerConfig) -> Value {
    match server.transport {
        TransportKind::Stdio => {
            let mut object = Map::new();
            if let Some(stdio) = &server.stdio {
                object.insert("command".to_string(), Value::String(stdio.command.clone()));
                object.insert(
                    "args".to_string(),
                    Value::Array(
                        stdio
                            .args
                            .iter()
                            .map(|a| Value::String(a.clone()))
                            .collect(),
                    ),
                );
                let env = export_key_values(&stdio.env);
                if !env.is_empty() {
                    object.insert("env".to_string(), Value::Object(env));
                }
            }
            object.insert("type".to_string(), Value::String("stdio".to_string()));
            Value::Object(object)
        }
        TransportKind::StreamableHttp | TransportKind::Sse => {
            let mut object = Map::new();
            if let Some(http) = &server.http {
                object.insert("url".to_string(), Value::String(http.url.clone()));
                let headers = export_key_values(&http.headers);
                if !headers.is_empty() {
                    object.insert("headers".to_string(), Value::Object(headers));
                }
            }
            object.insert(
                "type".to_string(),
                Value::String(if server.transport == TransportKind::Sse {
                    "sse".to_string()
                } else {
                    "http".to_string()
                }),
            );
            Value::Object(object)
        }
    }
}

fn export_key_values(items: &[KeyValue]) -> Map<String, Value> {
    let mut map = Map::new();
    for item in items {
        let value = if item.secret {
            SECRET_PLACEHOLDER.to_string()
        } else {
            item.value.clone().unwrap_or_default()
        };
        map.insert(item.key.clone(), Value::String(value));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn detect_and_build_claude_format() {
        let root: Value = serde_json::from_str(
            r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","pkg","D:\\data"],"env":{"DEPLOY_TOKEN":"Bearer abc12345678","PLAIN":"value"}}}}"#,
        )
        .unwrap();
        let entries = detect_entries(&root).expect("应识别 mcpServers");
        assert_eq!(entries.len(), 1);
        let draft = build_draft(&entries[0].0, &entries[0].1).expect("应构建草稿");
        assert_eq!(draft.transport, TransportKind::Stdio);
        let env = &draft.stdio.as_ref().unwrap().env;
        assert!(env.iter().any(|kv| kv.key == "PLAIN" && kv.value.is_some()));
        assert!(
            env.iter()
                .any(|kv| kv.key == "DEPLOY_TOKEN" && kv.secret && kv.value.is_none())
        );
        assert_eq!(draft.secrets.len(), 1);
        assert_eq!(draft.secrets[0].action, SecretAction::Replace);
    }

    #[test]
    fn build_vscode_format_and_placeholder() {
        let root: Value = serde_json::from_str(
            r#"{"servers":{"remote":{"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"${MCPTESTER_SECRET}","X-Trace":"abc"}}}}"#,
        )
        .unwrap();
        let entries = detect_entries(&root).expect("应识别 servers");
        let draft = build_draft(&entries[0].0, &entries[0].1).expect("应构建草稿");
        assert_eq!(draft.transport, TransportKind::StreamableHttp);
        // 占位符 → 待配置（无明文、无 keyring 写入）
        assert!(draft.secrets.is_empty());
        let headers = &draft.http.as_ref().unwrap().headers;
        assert!(
            headers
                .iter()
                .any(|kv| kv.key == "Authorization" && kv.secret && kv.value.is_none())
        );
        assert!(
            headers
                .iter()
                .any(|kv| kv.key == "X-Trace" && !kv.secret && kv.value.as_deref() == Some("abc"))
        );
    }

    #[test]
    fn sensitive_heuristics() {
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("X-Api-Key"));
        assert!(is_sensitive_key("GITHUB_PAT")); // _pat 后缀命中
        assert!(!is_sensitive_key("monkey"));
        assert!(!is_sensitive_key("keyboard"));
        assert!(looks_like_secret_value("Bearer xyz123456"));
        assert!(looks_like_secret_value("sk-abcdefgh1234"));
        assert!(!looks_like_secret_value("plain-text"));
    }

    #[test]
    fn export_masks_secrets() {
        let now = Utc::now();
        let server = ServerConfig {
            id: "s1".to_string(),
            name: "demo".to_string(),
            transport: TransportKind::StreamableHttp,
            stdio: None,
            http: Some(HttpConfig {
                url: "https://example.com/mcp".to_string(),
                headers: vec![
                    KeyValue {
                        key: "Authorization".to_string(),
                        value: Some("****masked".to_string()),
                        secret: true,
                    },
                    KeyValue {
                        key: "X-Trace".to_string(),
                        value: Some("abc".to_string()),
                        secret: false,
                    },
                ],
                timeout_secs: None,
                allow_insecure_tls: false,
                oauth: None,
            }),
            protocol_version_pin: None,
            enabled: true,
            tags: Vec::new(),
            origin: None,
            created_at: now,
            updated_at: now,
            last_connected_at: None,
        };
        let text = build_export(&[server], "vscode").expect("导出失败");
        assert!(text.contains(SECRET_PLACEHOLDER));
        assert!(!text.contains("masked"));
        assert!(text.contains("\"X-Trace\": \"abc\""));
        assert!(build_export(&[], "unknown").is_err());
    }
}
