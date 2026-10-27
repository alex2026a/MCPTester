//! 开发用探测 CLI：连接 MCP 服务器并输出握手 / 接口列表 / 调用结果
//!
//! 用法：
//! ```text
//! probe [--era auto|modern|legacy] --stdio <command> [args...] [--tool <name>] [--args <json>]
//! probe --http <url> [--header K=V]... [--tool <name>] [--args <json>]
//! ```
//! 仅开发使用，不进入生产包。

use std::sync::Arc;

use chrono::Utc;
use mcptester_lib::mcp::connection::ConnectionHandle;
use mcptester_lib::mcp::models::{
    HttpConfig, KeyValue, RuntimeSettings, ServerConfig, StdioConfig, TransportKind,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(if args.is_empty() { 2 } else { 0 });
    }

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime 创建失败");
    match runtime.block_on(run(&args)) {
        Ok(()) => {}
        Err(message) => {
            eprintln!("probe 失败：{message}");
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!(
        "用法：\n  probe [--era auto|modern|legacy] --stdio <command> [args...] [--tool <name>] [--args <json>]\n  \
         probe --http <url> [--header K=V]... [--tool <name>] [--args <json>]\n\
         说明：--tool 发起一次工具调用并输出结果；缺省则只做连接 + 枚举。"
    );
}

async fn run(args: &[String]) -> Result<(), String> {
    // 解析参数
    let mut era = "auto".to_string();
    let mut mode: Option<&str> = None;
    let mut command: Option<String> = None;
    let mut stdio_args: Vec<String> = Vec::new();
    let mut url: Option<String> = None;
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut tool: Option<String> = None;
    let mut tool_args = serde_json::Value::Null;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--era" => {
                i += 1;
                era = args.get(i).cloned().unwrap_or_else(|| "auto".to_string());
            }
            "--stdio" => {
                mode = Some("stdio");
            }
            "--http" => {
                mode = Some("http");
                i += 1;
                url = args.get(i).cloned();
            }
            "--header" => {
                i += 1;
                if let Some(kv) = args.get(i)
                    && let Some((k, v)) = kv.split_once('=')
                {
                    headers.push(KeyValue {
                        key: k.to_string(),
                        value: Some(v.to_string()),
                        secret: false,
                    });
                }
            }
            "--tool" => {
                i += 1;
                tool = args.get(i).cloned();
            }
            "--args" => {
                i += 1;
                let raw = args.get(i).cloned().unwrap_or_default();
                tool_args =
                    serde_json::from_str(&raw).map_err(|e| format!("--args 不是合法 JSON：{e}"))?;
            }
            other => {
                if mode == Some("stdio") {
                    if command.is_none() {
                        command = Some(other.to_string());
                    } else {
                        stdio_args.push(other.to_string());
                    }
                } else if mode.is_none() && command.is_none() {
                    return Err(format!("未知参数：{other}"));
                } else {
                    stdio_args.push(other.to_string());
                }
            }
        }
        i += 1;
    }

    let settings = RuntimeSettings::default();
    let (transport, stdio, http) = match mode {
        Some("stdio") => {
            let command = command.ok_or_else(|| "--stdio 缺少命令".to_string())?;
            (
                TransportKind::Stdio,
                Some(StdioConfig {
                    command,
                    args: stdio_args,
                    env: Vec::new(),
                    cwd: None,
                    shell_wrap: Default::default(),
                }),
                None,
            )
        }
        Some("http") => {
            let url = url.ok_or_else(|| "--http 缺少 URL".to_string())?;
            (
                TransportKind::StreamableHttp,
                None,
                Some(HttpConfig {
                    url,
                    headers,
                    timeout_secs: None,
                    allow_insecure_tls: false,
                    oauth: None,
                }),
            )
        }
        _ => {
            print_usage();
            return Err("必须指定 --stdio 或 --http".to_string());
        }
    };

    let now = Utc::now();
    let config = ServerConfig {
        id: "probe".to_string(),
        name: "probe".to_string(),
        transport,
        stdio,
        http,
        protocol_version_pin: match era.as_str() {
            "modern" => Some("2026-07-28".to_string()),
            "legacy" => Some("2025-11-25".to_string()),
            _ => None,
        },
        enabled: true,
        tags: Vec::new(),
        origin: None,
        created_at: now,
        updated_at: now,
        last_connected_at: None,
    };

    let handle: Arc<ConnectionHandle> = ConnectionHandle::new(
        config.id.clone(),
        config.name.clone(),
        config.protocol_version_pin.clone(),
        config.transport.clone(),
        settings,
    );

    match &config.transport {
        TransportKind::Stdio => handle
            .connect_stdio(config.stdio.as_ref().expect("stdio 配置缺失"))
            .await
            .map_err(|e| e.message())?,
        TransportKind::StreamableHttp => handle
            .connect_http(config.http.as_ref().expect("http 配置缺失"), None)
            .await
            .map_err(|e| e.message())?,
        TransportKind::Sse => return Err("SSE 传输 M2b 提供".to_string()),
    }

    let state = handle.state().await;
    println!("== 连接状态 ==\n{state:?}");

    let snapshot = handle.capabilities(true).await.map_err(|e| e.message())?;
    println!(
        "== 接口 ==\ntools={} resources={} templates={} prompts={}",
        snapshot.tools.len(),
        snapshot.resources.len(),
        snapshot.resource_templates.len(),
        snapshot.prompts.len()
    );
    for tool in &snapshot.tools {
        println!("  tool: {}", tool.name);
    }

    if let Some(tool) = tool {
        let record = handle.call_tool(&tool, tool_args).await;
        println!("== 调用 {} ==", tool);
        println!(
            "status={:?} duration_ms={} error={:?}",
            record.status, record.duration_ms, record.error
        );
        if let Some(result) = record.result {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_default()
            );
        }
    }

    handle.disconnect().await;
    Ok(())
}
