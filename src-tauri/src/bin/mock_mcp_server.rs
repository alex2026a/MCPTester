//! 内置 mock MCP 服务器（测试与离线验收；见 05 文档 M2 任务 10）
//!
//! 用法：
//! ```text
//! mock-mcp-server --transport stdio [--era both|modern|legacy]
//! mock-mcp-server --transport http --port 18080 [--era ...] [--fastmcp-compat] [--token mock-token]
//! ```
//! 特性：
//! - 确定性工具（echo / add / structured / slow_echo / input_required）供调用与重放验收
//! - 资源与资源模板、提示词
//! - `subscriptions/listen` 列表变更推送（现代世代）
//! - `--era` 控制支持的协议版本，用于双世代协商测试
//! - `--fastmcp-compat`（v0.1.1）：模拟 FastMCP/Python 服务端行为——
//!   强制 `Authorization: Bearer {token}`；无会话的非 initialize 请求返回
//!   400 + `id:null` JSON-RPC 错误（"Missing session ID"）

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, InputRequiredResult, JsonObject,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, Role, ServerCapabilities, ServerConfig,
    SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData as McpError, RoleServer};
use serde_json::{Value, json};

#[derive(Clone)]
struct MockServer {
    era: Era,
    caps: CapsMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Both,
    Modern,
    Legacy,
}

/// 能力声明模式（用于验证客户端按 capabilities 过滤枚举）
#[derive(Clone, Copy, PartialEq, Eq)]
enum CapsMode {
    /// 声明 tools/resources/prompts，且全部列表方法可用
    All,
    /// 只声明 tools；resources/prompts 列表方法返回 -32601
    ToolsOnly,
    /// 声明 tools/resources/prompts，但 templates/list 返回 -32601
    NoTemplates,
    /// 不声明任何能力；工具列表仍可用，其余列表返回 -32601
    None,
}

impl Era {
    fn versions(self) -> Cow<'static, [ProtocolVersion]> {
        match self {
            Era::Both => Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS),
            Era::Modern => Cow::Owned(vec![ProtocolVersion::V_2026_07_28]),
            Era::Legacy => Cow::Owned(vec![
                ProtocolVersion::V_2025_11_25,
                ProtocolVersion::V_2025_06_18,
                ProtocolVersion::V_2025_03_26,
                ProtocolVersion::V_2024_11_05,
            ]),
        }
    }
}

fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(JsonObject::new()),
    }
}

fn read_only() -> ToolAnnotations {
    ToolAnnotations::from_raw(None, Some(true), Some(false), Some(true), Some(false))
}

fn tool_echo() -> Tool {
    let mut tool = Tool::new(
        "echo",
        "回显输入文本",
        schema(json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": "要回显的文本" } },
            "required": ["text"]
        })),
    );
    tool.title = Some("回显".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_add() -> Tool {
    let mut tool = Tool::new(
        "add",
        "计算 a + b",
        schema(json!({
            "type": "object",
            "properties": {
                "a": { "type": "number" },
                "b": { "type": "number" }
            },
            "required": ["a", "b"]
        })),
    );
    tool.title = Some("求和".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_structured() -> Tool {
    let mut tool = Tool::new(
        "structured",
        "返回结构化内容 + 文本内容",
        schema(json!({ "type": "object", "properties": {}, "required": [] })),
    );
    tool.title = Some("结构化输出".to_string());
    tool.annotations = Some(read_only());
    tool.output_schema = Some(schema(json!({
        "type": "object",
        "required": ["ok", "items"],
        "properties": {
            "ok": { "type": "boolean" },
            "items": { "type": "array", "items": { "type": "integer" } }
        }
    })));
    tool
}

fn tool_slow_echo() -> Tool {
    let mut tool = Tool::new(
        "slow_echo",
        "延迟 delay_ms 后回显；用于超时与取消测试",
        schema(json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "delay_ms": { "type": "integer", "minimum": 0 }
            },
            "required": ["text"]
        })),
    );
    tool.title = Some("慢回显".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_input_required() -> Tool {
    let mut tool = Tool::new(
        "input_required",
        "返回 resultType=input_required，用于验证 v0.1 的 MRTR 处理",
        schema(json!({ "type": "object", "properties": {}, "required": [] })),
    );
    tool.title = Some("需要交互输入（MRTR）".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_bounded() -> Tool {
    let mut tool = Tool::new(
        "bounded",
        "边界工具：字符串/数字/数组/枚举/嵌套；非法参数返回 isError=true",
        schema(json!({
            "type": "object",
            "required": ["name", "count", "tags"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 2, "maxLength": 20 },
                "count": { "type": "integer", "minimum": 1, "maximum": 5 },
                "tags": {
                    "type": "array", "minItems": 1, "maxItems": 3,
                    "items": { "type": "string", "minLength": 1 }
                },
                "mode": { "type": "string", "enum": ["a", "b"] },
                "nested": {
                    "type": "object",
                    "required": ["id"],
                    "properties": { "id": { "type": "integer", "minimum": 1 } }
                }
            }
        })),
    );
    tool.title = Some("边界工具".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_strict() -> Tool {
    let mut tool = Tool::new(
        "strict",
        "严格工具：缺失必填参数返回 JSON-RPC error",
        schema(json!({
            "type": "object",
            "required": ["value"],
            "properties": { "value": { "type": "string", "minLength": 1 } }
        })),
    );
    tool.title = Some("严格校验".to_string());
    tool.annotations = Some(read_only());
    tool
}

fn tool_destroy() -> Tool {
    let mut tool = Tool::new(
        "destroy",
        "破坏性工具（默认不运行）",
        schema(json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })),
    );
    tool.title = Some("破坏性操作".to_string());
    tool.annotations = Some(ToolAnnotations::from_raw(
        None,
        Some(false),
        Some(true),
        Some(true),
        Some(false),
    ));
    tool
}

fn tool_write() -> Tool {
    let mut tool = Tool::new(
        "write_value",
        "写操作但不具破坏性",
        schema(json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        })),
    );
    tool.title = Some("写入值".to_string());
    tool.annotations = Some(ToolAnnotations::from_raw(
        None,
        Some(false),
        Some(false),
        Some(false),
        Some(false),
    ));
    tool
}

fn tool_unknown() -> Tool {
    let mut tool = Tool::new(
        "unknown_risk",
        "未声明注解的未知风险工具",
        schema(json!({ "type": "object", "properties": {} })),
    );
    tool.title = Some("未知风险".to_string());
    tool
}

fn tool_no_schema() -> Tool {
    let mut tool = Tool::new(
        "no_schema",
        "无可用 inputSchema",
        schema(json!({})),
    );
    tool.title = Some("无 Schema".to_string());
    tool.annotations = Some(read_only());
    tool
}

impl ServerHandler for MockServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = match self.caps {
            CapsMode::All | CapsMode::NoTemplates => ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .enable_prompts()
                .build(),
            CapsMode::ToolsOnly => ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
            CapsMode::None => ServerCapabilities::default(),
        };
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new(
                "mock-mcp-server",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("MCP Tester 内置 mock 服务器（确定性输出，供测试与离线验收）")
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.era.versions()
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(vec![
            tool_echo(),
            tool_add(),
            tool_structured(),
            tool_slow_echo(),
            tool_input_required(),
            tool_bounded(),
            tool_strict(),
            tool_destroy(),
            tool_write(),
            tool_unknown(),
            tool_no_schema(),
        ])))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        match name.as_str() {
            "echo" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
                Ok(CallToolResult::success(vec![ContentBlock::text(format!("echo: {text}"))]).into())
            }
            "add" => {
                let a = args.get("a").and_then(Value::as_f64).unwrap_or(0.0);
                let b = args.get("b").and_then(Value::as_f64).unwrap_or(0.0);
                let sum = a + b;
                let mut result = CallToolResult::structured(json!({ "sum": sum }));
                result.content = vec![ContentBlock::text(format!("{sum}"))];
                Ok(result.into())
            }
            "structured" => {
                let mut result =
                    CallToolResult::structured(json!({ "ok": true, "items": [1, 2, 3] }));
                result.content = vec![ContentBlock::text("结构化结果：3 项")];
                Ok(result.into())
            }
            "slow_echo" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let delay = args.get("delay_ms").and_then(Value::as_u64).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                Ok(CallToolResult::success(vec![ContentBlock::text(format!("echo: {text}"))]).into())
            }
            "input_required" => {
                // MRTR：要求客户端提供交互输入（v0.1 期望：明确报错，不自动重试）
                Ok(CallToolResponse::InputRequired(
                    InputRequiredResult::from_request_state("mock-state"),
                ))
            }
            "bounded" => {
                let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
                let count = args.get("count").and_then(Value::as_i64).unwrap_or(0);
                let tags = args.get("tags").and_then(Value::as_array);
                let valid = name.chars().count() >= 2
                    && name.chars().count() <= 20
                    && (1..=5).contains(&count)
                    && tags.map(|items| (1..=3).contains(&items.len())).unwrap_or(false);
                if valid {
                    let mut result = CallToolResult::structured(json!({
                        "ok": true,
                        "name": name,
                        "count": count
                    }));
                    result.content = vec![ContentBlock::text(format!("bounded: {name}/{count}"))];
                    Ok(result.into())
                } else {
                    let mut result = CallToolResult::success(vec![ContentBlock::text(
                        "参数不合法：name/count/tags 超出约束",
                    )]);
                    result.is_error = Some(true);
                    Ok(result.into())
                }
            }
            "strict" => {
                let value = args.get("value").and_then(Value::as_str).unwrap_or_default();
                if value.is_empty() {
                    Err(McpError::new(
                        rmcp::model::ErrorCode::INVALID_PARAMS,
                        "value 不能为空",
                        None,
                    ))
                } else {
                    Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                        "strict: {value}"
                    ))])
                    .into())
                }
            }
            "destroy" => Ok(CallToolResult::success(vec![ContentBlock::text(
                "destroyed（mock，不产生真实副作用）",
            )])
            .into()),
            "write_value" => Ok(CallToolResult::success(vec![ContentBlock::text(
                "written（mock，不产生真实副作用）",
            )])
            .into()),
            "unknown_risk" => Ok(CallToolResult::success(vec![ContentBlock::text(
                "unknown risk executed",
            )])
            .into()),
            "no_schema" => Ok(CallToolResult::success(vec![ContentBlock::text(
                "no schema executed",
            )])
            .into()),
            other => Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("未知工具：{other}"),
                None,
            )),
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        if matches!(self.caps, CapsMode::ToolsOnly | CapsMode::None) {
            return std::future::ready(Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                "Method not found",
                None,
            )));
        }
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("mock://hello", "hello").with_description("固定问候资源"),
        ])))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        if matches!(
            self.caps,
            CapsMode::ToolsOnly | CapsMode::NoTemplates | CapsMode::None
        ) {
            return std::future::ready(Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                "Method not found",
                None,
            )));
        }
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("mock://item/{id}", "item")
                .with_description("按 id 读取条目（模板）"),
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + Send + '_ {
        let uri = request.uri.clone();
        let content = if uri == "mock://hello" {
            "hello from mock".to_string()
        } else if let Some(id) = uri.strip_prefix("mock://item/") {
            format!("item:{id}")
        } else {
            return std::future::ready(Err(McpError::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("未知资源：{uri}"),
                None,
            )));
        };
        std::future::ready(Ok(ReadResourceResult::new(vec![ResourceContents::text(
            content, uri,
        )])
        .into()))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        if matches!(self.caps, CapsMode::ToolsOnly | CapsMode::None) {
            return std::future::ready(Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                "Method not found",
                None,
            )));
        }
        std::future::ready(Ok(ListPromptsResult::with_all_items(vec![Prompt::new(
            "greet",
            Some("问候提示词"),
            Some(vec![
                PromptArgument::new("name")
                    .with_description("要问候的名字")
                    .with_required(true),
            ]),
        )])))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, McpError>> + Send + '_ {
        let name = request
            .arguments
            .as_ref()
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("世界")
            .to_string();
        let result = GetPromptResult::new(vec![PromptMessage::new_text(
            Role::User,
            format!("你好，{name}！"),
        )])
        .with_description("mock 问候提示词");
        std::future::ready(Ok(result.into()))
    }

    /// 现代世代：接受全部列表变更订阅
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    /// 现代世代：订阅建立后推送一次工具列表变更（验证客户端 listen 扇入）
    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = context
            .request_context()
            .peer
            .notify_tool_list_changed()
            .await;
        context.cancelled().await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime 创建失败");
    runtime.block_on(async_main());
}

async fn async_main() {
    let mut transport = "stdio".to_string();
    let mut era = Era::Both;
    let mut caps = CapsMode::All;
    let mut port: u16 = 18080;
    let mut exit_after_ms: u64 = 0;
    let mut fastmcp_compat: Option<String> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--transport" => {
                i += 1;
                transport = args.get(i).cloned().unwrap_or(transport);
            }
            "--era" => {
                i += 1;
                era = match args.get(i).map(String::as_str) {
                    Some("modern") => Era::Modern,
                    Some("legacy") => Era::Legacy,
                    _ => Era::Both,
                };
            }
            "--caps" => {
                i += 1;
                caps = match args.get(i).map(String::as_str) {
                    Some("tools-only") => CapsMode::ToolsOnly,
                    Some("no-templates") => CapsMode::NoTemplates,
                    Some("none") => CapsMode::None,
                    _ => CapsMode::All,
                };
            }
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|p| p.parse().ok()).unwrap_or(port);
            }
            "--exit-after-ms" => {
                // 测试用：指定毫秒后进程自行退出（验证客户端 lost 检测）
                i += 1;
                exit_after_ms = args.get(i).and_then(|p| p.parse().ok()).unwrap_or(0);
            }
            "--fastmcp-compat" => {
                // FastMCP 兼容模式：启用鉴权与会话语义模拟（默认 token：mock-token）
                fastmcp_compat = Some("mock-token".to_string());
            }
            "--token" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    fastmcp_compat = Some(value.clone());
                }
            }
            "--help" | "-h" => {
                println!(
                    "用法：mock-mcp-server --transport stdio|http|sse [--era both|modern|legacy] [--caps all|tools-only|no-templates|none] [--port N] [--exit-after-ms N] [--fastmcp-compat] [--token T]"
                );
                return;
            }
            _ => {}
        }
        i += 1;
    }

    if exit_after_ms > 0 {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(exit_after_ms)).await;
            std::process::exit(0);
        });
    }

    match transport.as_str() {
        "stdio" => run_stdio(era, caps).await,
        "http" => run_http(era, port, fastmcp_compat, caps).await,
        "sse" => run_sse(port).await,
        other => {
            eprintln!("不支持的传输：{other}");
            std::process::exit(2);
        }
    }
}

async fn run_stdio(era: Era, caps: CapsMode) {
    use rmcp::ServiceExt;
    let service = MockServer { era, caps }
        .serve(rmcp::transport::io::stdio())
        .await
        .expect("mock server 启动失败");
    let _ = service.waiting().await;
}

// ---------------------------------------------------------------------------
// 遗留 HTTP+SSE 服务端（自研：rmcp 只提供 Streamable HTTP 服务端）
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SseState {
    sessions: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>,
}

async fn run_sse(port: u16) {
    use axum::routing::{get, post};

    let state = SseState {
        sessions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };
    let app = axum::Router::new()
        .route("/sse", get(sse_handler))
        .route("/messages", post(sse_messages_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("mock SSE 端口绑定失败");
    println!("mock-mcp-server (sse) listening on http://127.0.0.1:{port}/sse");
    axum::serve(listener, app).await.expect("mock SSE 服务失败");
}

async fn sse_handler(
    axum::extract::State(state): axum::extract::State<SseState>,
) -> axum::response::sse::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::convert::Infallible;

    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    state.sessions.lock().await.insert(session_id.clone(), tx);

    let endpoint = format!("/messages?sessionId={session_id}");
    let first = futures::stream::once(async move {
        Ok::<_, Infallible>(Event::default().event("endpoint").data(endpoint))
    });
    let rest = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|data| (Ok::<_, Infallible>(Event::default().data(data)), rx))
    });
    Sse::new(futures::StreamExt::chain(first, rest)).keep_alive(KeepAlive::default())
}

async fn sse_messages_handler(
    axum::extract::State(state): axum::extract::State<SseState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
    body: String,
) -> axum::http::StatusCode {
    let request: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => return axum::http::StatusCode::BAD_REQUEST,
    };
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 通知：无 id，直接忽略（仅记录日志）
    let Some(id) = request.get("id").cloned() else {
        eprintln!("[mock-sse] 收到通知：{method}");
        return axum::http::StatusCode::ACCEPTED;
    };

    let response = sse_dispatch(&method, &request, id);
    if let Some(session_id) = query.get("sessionId") {
        let sessions = state.sessions.lock().await;
        if let Some(sender) = sessions.get(session_id) {
            let _ = sender.send(response.to_string());
        }
    }
    axum::http::StatusCode::ACCEPTED
}

/// 旧世代 JSON-RPC 分发（SSE 传输仅用于旧世代；server/discover 返回方法不存在）
fn sse_dispatch(method: &str, request: &Value, id: Value) -> Value {
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let ok = |result: Value| serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let err = |code: i64, message: &str| {
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
    };
    let schema = |value: Value| value;

    match method {
        "initialize" => {
            // 回显客户端请求版本（SSE 属旧世代）
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-11-25");
            let version = if requested == "2026-07-28" {
                "2025-11-25"
            } else {
                requested
            };
            ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": true }, "resources": {}, "prompts": {} },
                "serverInfo": { "name": "mock-mcp-server", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "MCP Tester 内置 mock 服务器（SSE 模式）"
            }))
        }
        "server/discover" => err(-32601, "Method not found（SSE 为旧世代传输）"),
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({ "tools": [
            { "name": "echo", "description": "回显输入文本",
              "inputSchema": schema(json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] })) },
            { "name": "add", "description": "计算 a + b",
              "inputSchema": schema(json!({ "type": "object", "properties": { "a": { "type": "number" }, "b": { "type": "number" } }, "required": ["a", "b"] })) }
        ] })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match name {
                "echo" => {
                    let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
                    ok(json!({ "content": [{ "type": "text", "text": format!("echo: {text}") }] }))
                }
                "add" => {
                    let a = args.get("a").and_then(Value::as_f64).unwrap_or(0.0);
                    let b = args.get("b").and_then(Value::as_f64).unwrap_or(0.0);
                    ok(json!({
                        "content": [{ "type": "text", "text": format!("{}", a + b) }],
                        "structuredContent": { "sum": a + b }
                    }))
                }
                other => err(-32601, &format!("未知工具：{other}")),
            }
        }
        "resources/list" => ok(json!({ "resources": [
            { "uri": "mock://hello", "name": "hello", "description": "固定问候资源" }
        ] })),
        "resources/templates/list" => ok(json!({ "resourceTemplates": [
            { "uriTemplate": "mock://item/{id}", "name": "item", "description": "按 id 读取条目（模板）" }
        ] })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default();
            let text = if uri == "mock://hello" {
                "hello from mock".to_string()
            } else if let Some(id) = uri.strip_prefix("mock://item/") {
                format!("item:{id}")
            } else {
                return err(-32602, &format!("未知资源：{uri}"));
            };
            ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/plain", "text": text }] }))
        }
        "prompts/list" => ok(json!({ "prompts": [
            { "name": "greet", "description": "问候提示词",
              "arguments": [{ "name": "name", "description": "要问候的名字", "required": true }] }
        ] })),
        "prompts/get" => {
            let name = params.get("arguments").and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or("世界");
            ok(json!({
                "description": "mock 问候提示词",
                "messages": [{ "role": "user", "content": { "type": "text", "text": format!("你好，{name}！") } }]
            }))
        }
        other => err(-32601, &format!("Method not found: {other}")),
    }
}

async fn run_http(era: Era, port: u16, fastmcp_compat: Option<String>, caps: CapsMode) {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let service = StreamableHttpService::new(
        move || Ok(MockServer { era, caps }),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let mut app = axum::Router::new().nest_service("/mcp", service);
    if let Some(token) = fastmcp_compat {
        // v0.1.1：FastMCP 兼容层（鉴权 + 无会话 400），用于客户端回归测试
        app = app.layer(axum::middleware::from_fn_with_state(
            FastmcpCompat {
                token: Arc::from(token.as_str()),
            },
            fastmcp_compat_layer,
        ));
        println!("mock-mcp-server (http, fastmcp-compat) listening on http://127.0.0.1:{port}/mcp");
    } else {
        println!("mock-mcp-server (http) listening on http://127.0.0.1:{port}/mcp");
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("mock HTTP 端口绑定失败");
    axum::serve(listener, app).await.expect("mock HTTP 服务失败");
}

// ---------------------------------------------------------------------------
// FastMCP 兼容层（v0.1.1 回归测试：401 鉴权 / 无会话 400）
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FastmcpCompat {
    token: Arc<str>,
}

/// 模拟 FastMCP/Python 服务端行为：
/// 1. 强制 `Authorization: Bearer {token}`，否则 401 + `WWW-Authenticate`；
/// 2. 无 `mcp-session-id` 的非 initialize 请求 → 400 + `id:null` JSON-RPC 错误。
async fn fastmcp_compat_layer(
    axum::extract::State(compat): axum::extract::State<FastmcpCompat>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // 1) 鉴权
    let expected = format!("Bearer {}", compat.token);
    let authorized = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == expected)
        .unwrap_or(false);
    if !authorized {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(
                axum::http::header::WWW_AUTHENTICATE,
                "Bearer error=\"invalid_token\", error_description=\"Authentication required\"",
            )],
            axum::Json(serde_json::json!({
                "error": "invalid_token",
                "error_description": "Authentication required"
            })),
        )
            .into_response();
    }

    // 2) 会话语义（FastMCP：非 initialize 请求缺会话 → 400，id 置空）
    if request.method() == axum::http::Method::POST
        && !request.headers().contains_key("mcp-session-id")
    {
        let (parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({ "error": "invalid_body" })),
                )
                    .into_response();
            }
        };
        let is_non_initialize_request = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .map(|value| {
                let has_id = value.get("id").map(|id| !id.is_null()).unwrap_or(false);
                let method = value.get("method").and_then(Value::as_str);
                has_id && method != Some("initialize")
            })
            .unwrap_or(false);
        if is_non_initialize_request {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32600, "message": "Bad Request: Missing session ID" }
                })),
            )
                .into_response();
        }
        let request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
        return next.run(request).await;
    }

    next.run(request).await
}
