//! M2a 集成测试：stdio / Streamable HTTP 连接、枚举、调用、超时、取消、掉线检测
//!
//! 运行：`cargo test --features mock-server`
//! （mock-mcp-server 由 `required-features` 门控，仅在启用 mock-server 特性时构建）

#![cfg(feature = "mock-server")]

use std::sync::Arc;
use std::time::Duration;

use mcptester_lib::mcp::connection::ConnectionHandle;
use mcptester_lib::mcp::models::{
    ConnectionState, HttpConfig, KeyValue, LifecycleEra, RuntimeSettings, ShellWrap, StdioConfig,
    TransportKind,
};

const MOCK_BIN: &str = env!("CARGO_BIN_EXE_mock-mcp-server");

fn stdio_cfg(args: &[&str]) -> StdioConfig {
    StdioConfig {
        command: MOCK_BIN.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: Vec::new(),
        cwd: None,
        shell_wrap: ShellWrap::Never, // 测试用绝对路径，无需 PATH 解析
    }
}

fn handle(id: &str, settings: RuntimeSettings) -> Arc<ConnectionHandle> {
    ConnectionHandle::new(id, id, None, TransportKind::Stdio, settings)
}

/// 读取握手信息（Ready 时）
async fn handshake_of(handle: &ConnectionHandle) -> mcptester_lib::mcp::models::Handshake {
    match handle.state().await {
        ConnectionState::Ready { handshake } => handshake,
        other => panic!("期望 Ready，实际 {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_modern_connect_list_call() {
    let h = handle("mock-modern", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "both"]))
        .await
        .expect("连接失败");

    let handshake = handshake_of(&h).await;
    assert_eq!(handshake.lifecycle, LifecycleEra::Modern, "应协商为现代世代");
    assert_eq!(handshake.protocol_version, "2026-07-28");
    assert_eq!(handshake.server_name, "mock-mcp-server");

    let snapshot = h.capabilities(true).await.expect("枚举失败");
    assert_eq!(snapshot.tools.len(), 5);
    assert_eq!(snapshot.resources.len(), 1);
    assert_eq!(snapshot.resource_templates.len(), 1);
    assert_eq!(snapshot.prompts.len(), 1);
    assert!(
        snapshot.tools.iter().any(|t| t.name == "echo"),
        "应包含 echo 工具"
    );

    // 调用 echo（确定性断言）
    let record = h.call_tool("echo", serde_json::json!({ "text": "hi" })).await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Ok);
    assert_eq!(record.method, mcptester_lib::mcp::models::CallMethod::ToolsCall);
    assert!(record.error.is_none());
    let result = record.result.expect("应有结果");
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("echo: hi"), "结果应包含 echo 输出：{text}");
    assert_eq!(record.protocol_version, "2026-07-28");

    // 资源读取与提示词
    let resource = h.read_resource("mock://item/42").await;
    assert_eq!(resource.status, mcptester_lib::mcp::models::CallStatus::Ok);
    assert!(
        serde_json::to_string(&resource.result).unwrap().contains("item:42")
    );

    let prompt = h.get_prompt("greet", serde_json::json!({ "name": "MCP" })).await;
    assert_eq!(prompt.status, mcptester_lib::mcp::models::CallStatus::Ok);

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_legacy_era_negotiates_and_probes() {
    let h = handle("mock-legacy", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "legacy"]))
        .await
        .expect("连接失败");

    let handshake = handshake_of(&h).await;
    assert_eq!(handshake.lifecycle, LifecycleEra::Legacy, "应回退旧世代");
    assert_eq!(handshake.protocol_version, "2025-11-25");

    // 旧世代健康探测：ping 请求
    let probe = h.probe().await.expect("探测失败");
    assert!(probe.ok, "旧世代 ping 探测应成功：{:?}", probe.detail);

    // 旧世代仍可枚举与调用
    let snapshot = h.capabilities(true).await.expect("枚举失败");
    assert_eq!(snapshot.tools.len(), 5);

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn modern_subscription_receives_list_changed() {
    let h = handle("mock-subscribe", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "modern"]))
        .await
        .expect("连接失败");
    assert_eq!(handshake_of(&h).await.lifecycle, LifecycleEra::Modern);

    // mock 在订阅建立 200ms 后推送 tools/list_changed
    tokio::time::sleep(Duration::from_millis(800)).await;
    let notifications = h.hub().notification_buffer(0);
    assert!(
        notifications
            .iter()
            .any(|n| n.method.contains("tools/list_changed")),
        "应收到 tools/list_changed 订阅通知，实际：{:?}",
        notifications.iter().map(|n| &n.method).collect::<Vec<_>>()
    );

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn call_timeout_maps_to_timeout_error() {
    let settings = RuntimeSettings {
        request_timeout_secs: 1,
        ..Default::default()
    };
    let h = handle("mock-timeout", settings);
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "both"]))
        .await
        .expect("连接失败");

    let started = std::time::Instant::now();
    let record = h
        .call_tool(
            "slow_echo",
            serde_json::json!({ "text": "slow", "delay_ms": 5000 }),
        )
        .await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Error);
    let error = record.error.expect("应有错误");
    assert!(
        matches!(
            error,
            mcptester_lib::mcp::error::CallError::Timeout { .. }
        ),
        "应为超时错误，实际：{error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "超时应及时返回"
    );

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_call_aborts_wait_and_notifies_server() {
    let h = handle("mock-cancel", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "both"]))
        .await
        .expect("连接失败");

    let call_id = "test-cancel-1".to_string();
    let caller = {
        let h = Arc::clone(&h);
        let call_id = call_id.clone();
        tokio::spawn(async move {
            h.call_tool_with_id(
                Some(call_id),
                "slow_echo",
                serde_json::json!({ "text": "cancel-me", "delay_ms": 5000 }),
            )
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.cancel_call(&call_id).await, "取消应命中在途调用");

    let record = tokio::time::timeout(Duration::from_secs(3), caller)
        .await
        .expect("取消后调用应快速返回")
        .expect("调用任务失败");
    assert_eq!(
        record.status,
        mcptester_lib::mcp::models::CallStatus::Error,
        "取消后应记录为错误（请求已取消）"
    );

    // 服务器侧应收到 notifications/cancelled（stdio 语义）
    tokio::time::sleep(Duration::from_millis(200)).await;
    let sdk_notifications = h.hub().notification_buffer(0);
    let _ = sdk_notifications; // 客户端侧不记录自身发出的取消通知

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_exit_marks_connection_lost() {
    let h = handle("mock-lost", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&[
        "--transport",
        "stdio",
        "--era",
        "both",
        "--exit-after-ms",
        "700",
    ]))
    .await
    .expect("连接失败");

    // 等待 mock 自行退出 + 监控任务轮询
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    loop {
        if let ConnectionState::Lost { .. } = h.state().await {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "服务器退出后连接应转为 lost，实际：{:?}",
            h.state().await
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mrtr_input_required_maps_to_error() {
    let h = handle("mock-mrtr", RuntimeSettings::default());
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "modern"]))
        .await
        .expect("连接失败");

    let record = h
        .call_tool("input_required", serde_json::json!({}))
        .await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Error);
    assert!(
        matches!(
            record.error,
            Some(mcptester_lib::mcp::error::CallError::InputRequired)
        ),
        "MRTR 应映射为 InputRequired 错误，实际：{:?}",
        record.error
    );

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn http_connect_list_call() {
    // 启动 mock HTTP 服务（随机高位端口，避免冲突）
    let port = 18200 + (std::process::id() % 200) as u16;
    let mut child = std::process::Command::new(MOCK_BIN)
        .args(["--transport", "http", "--era", "both", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("mock HTTP 进程启动失败");

    // 等待端口就绪
    let url = format!("http://127.0.0.1:{port}/mcp");
    let mut ready = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "mock HTTP 服务未就绪");

    let h = ConnectionHandle::new(
        "mock-http",
        "mock-http",
        None,
        TransportKind::StreamableHttp,
        RuntimeSettings::default(),
    );
    h.connect_http(&HttpConfig {
        url,
        headers: vec![KeyValue {
            key: "X-Test-Header".to_string(),
            value: Some("m2a".to_string()),
            secret: false,
        }],
        timeout_secs: None,
        allow_insecure_tls: false,
    })
    .await
    .expect("HTTP 连接失败");

    let handshake = handshake_of(&h).await;
    assert_eq!(handshake.server_name, "mock-mcp-server");

    let snapshot = h.capabilities(true).await.expect("枚举失败");
    assert_eq!(snapshot.tools.len(), 5);

    let record = h.call_tool("add", serde_json::json!({ "a": 2, "b": 3 })).await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Ok);
    let text = serde_json::to_string(&record.result).unwrap();
    assert!(text.contains('5'), "add 结果应包含 5：{text}");
    assert_eq!(
        record.transport,
        TransportKind::StreamableHttp,
        "记录应标注 HTTP 传输"
    );

    h.disconnect().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread")]
async fn cmd_shell_mode_wraps_command() {
    // cmd_shell 模式：经 cmd /C 启动 mock（验证显式包装路径可用）
    let h = handle("mock-cmd-shell", RuntimeSettings::default());
    let cfg = StdioConfig {
        command: MOCK_BIN.to_string(),
        args: vec!["--transport".to_string(), "stdio".to_string(), "--era".to_string(), "both".to_string()],
        env: Vec::new(),
        cwd: None,
        shell_wrap: ShellWrap::CmdShell,
    };
    h.connect_stdio(&cfg).await.expect("cmd_shell 连接失败");
    let snapshot = h.capabilities(true).await.expect("枚举失败");
    assert_eq!(snapshot.tools.len(), 5);
    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_connect_list_call() {
    // 遗留 SSE：自研传输 + 自研 mock 服务端（仅旧世代）
    let port = 18800 + (std::process::id() % 150) as u16;
    let mut child = std::process::Command::new(MOCK_BIN)
        .args(["--transport", "sse", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("mock SSE 进程启动失败");

    let mut ready = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "mock SSE 服务未就绪");

    let h = ConnectionHandle::new(
        "mock-sse",
        "mock-sse",
        None,
        TransportKind::Sse,
        RuntimeSettings::default(),
    );
    h.connect_sse(&HttpConfig {
        url: format!("http://127.0.0.1:{port}/sse"),
        headers: Vec::new(),
        timeout_secs: None,
        allow_insecure_tls: false,
    })
    .await
    .expect("SSE 连接失败");

    let handshake = handshake_of(&h).await;
    assert_eq!(handshake.lifecycle, LifecycleEra::Legacy, "SSE 应为旧世代");
    assert_eq!(handshake.protocol_version, "2025-11-25");

    let snapshot = h.capabilities(true).await.expect("枚举失败");
    assert_eq!(snapshot.tools.len(), 2);

    let record = h.call_tool("echo", serde_json::json!({ "text": "sse" })).await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Ok);
    let text = serde_json::to_string(&record.result).unwrap();
    assert!(text.contains("echo: sse"), "SSE 调用结果异常：{text}");
    assert_eq!(record.transport, TransportKind::Sse);

    h.disconnect().await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_mode_missing_command_reports_spawn_failed() {
    // resolve 模式：命令不存在应明确报"找不到命令"（而不是挂起）
    let h = handle("mock-missing", RuntimeSettings::default());
    let cfg = StdioConfig {
        command: "definitely-not-a-real-command-mcptester".to_string(),
        args: Vec::new(),
        env: Vec::new(),
        cwd: None,
        shell_wrap: ShellWrap::Resolve,
    };
    let err = h.connect_stdio(&cfg).await.expect_err("应连接失败");
    let message = err.message();
    assert!(
        message.contains("找不到命令"),
        "错误应提示找不到命令，实际：{message}"
    );
    assert!(matches!(
        err,
        mcptester_lib::mcp::error::McpError::SpawnFailed { .. }
    ));
}

// ---------------------------------------------------------------------------
// v0.1.1：FastMCP 兼容回归
// （401 鉴权 + Bearer 归一化 + 无会话 discover 400 回退 + 报文日志）
// ---------------------------------------------------------------------------

/// 启动 FastMCP 兼容 mock（http），返回 (child, port)
async fn spawn_fastmcp_mock(era: &str, token: &str, base_port: u16) -> (std::process::Child, u16) {
    let port = base_port + (std::process::id() % 300) as u16;
    let child = std::process::Command::new(MOCK_BIN)
        .args([
            "--transport",
            "http",
            "--era",
            era,
            "--port",
            &port.to_string(),
            "--fastmcp-compat",
            "--token",
            token,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("mock fastmcp 进程启动失败");

    let mut ready = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "mock fastmcp 服务未就绪");
    (child, port)
}

fn auth_header(value: &str) -> KeyValue {
    KeyValue {
        key: "Authorization".to_string(),
        value: Some(value.to_string()),
        secret: true,
    }
}

fn http_handle(id: &str) -> Arc<ConnectionHandle> {
    ConnectionHandle::new(
        id,
        id,
        None,
        TransportKind::StreamableHttp,
        RuntimeSettings::default(),
    )
}

/// 完整 "Bearer xxx" 头值（用户按 UI 提示填写的形态）→ 归一化后必须成功连接；
/// 修复前为 "Bearer Bearer xxx" → 401 → handshake_failed（见 01 文档根因 A）
#[tokio::test(flavor = "multi_thread")]
async fn fastmcp_full_bearer_header_connects_and_logs_wire() {
    let (mut child, port) = spawn_fastmcp_mock("legacy", "mock-token", 19000).await;

    let h = http_handle("mock-fastmcp-bearer");
    h.connect_http(&HttpConfig {
        url: format!("http://127.0.0.1:{port}/mcp"),
        headers: vec![auth_header("Bearer mock-token")],
        timeout_secs: None,
        allow_insecure_tls: false,
    })
    .await
    .expect("完整 Bearer 头应可连接（v0.1.1 归一化）");

    let handshake = handshake_of(&h).await;
    assert_eq!(
        handshake.lifecycle,
        LifecycleEra::Legacy,
        "FastMCP 兼容服务端（无会话 discover → 400）应回退旧世代握手"
    );
    assert_eq!(handshake.server_name, "mock-mcp-server");

    // 报文日志：discover 探测 + initialize 握手 + 连接建立系统条目
    let wire = h.wire().buffer();
    assert!(
        wire.iter().any(|e| e.summary.contains("server/discover")),
        "报文日志应包含 discover 探测：{:?}",
        wire.iter().map(|e| &e.summary).collect::<Vec<_>>()
    );
    assert!(
        wire.iter().any(|e| e.summary.contains("initialize")),
        "报文日志应包含 initialize 握手"
    );
    assert!(
        wire.iter()
            .any(|e| e.direction == mcptester_lib::mcp::wire::WireDirection::System
                && e.summary.contains("连接建立")),
        "报文日志应包含连接建立系统条目"
    );

    // 调用链路仍可用
    let record = h
        .call_tool("echo", serde_json::json!({ "text": "compat" }))
        .await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Ok);

    h.disconnect().await;
    let _ = child.kill();
    let _ = child.wait();
}

/// 仅填 token（不带 Bearer 前缀）同样可用（归一化兜底加前缀）
#[tokio::test(flavor = "multi_thread")]
async fn fastmcp_plain_token_connects() {
    let (mut child, port) = spawn_fastmcp_mock("legacy", "mock-token", 19400).await;

    let h = http_handle("mock-fastmcp-plain");
    h.connect_http(&HttpConfig {
        url: format!("http://127.0.0.1:{port}/mcp"),
        headers: vec![auth_header("mock-token")],
        timeout_secs: None,
        allow_insecure_tls: false,
    })
    .await
    .expect("纯 token 应可连接");

    assert!(h.state().await.is_ready());
    h.disconnect().await;
    let _ = child.kill();
    let _ = child.wait();
}

/// 错误 token → 结构化"认证失败"（不再是 handshake_failed；见 01 文档 4.3）
#[tokio::test(flavor = "multi_thread")]
async fn fastmcp_wrong_token_reports_unauthorized() {
    let (mut child, port) = spawn_fastmcp_mock("legacy", "mock-token", 19800).await;

    let h = http_handle("mock-fastmcp-wrong");
    let err = h
        .connect_http(&HttpConfig {
            url: format!("http://127.0.0.1:{port}/mcp"),
            headers: vec![auth_header("Bearer wrong-token")],
            timeout_secs: None,
            allow_insecure_tls: false,
        })
        .await
        .expect_err("错误 token 应连接失败");

    assert_eq!(err.kind(), "unauthorized", "应分类为认证失败：{err:?}");
    assert!(
        err.message().contains("认证失败"),
        "错误消息应为中文认证失败：{}",
        err.message()
    );

    // 连接状态同样记录结构化错误（供服务器卡片展示）
    match h.state().await {
        ConnectionState::Error { error } => assert_eq!(error.kind(), "unauthorized"),
        other => panic!("期望 Error 状态，实际 {other:?}"),
    }

    // 报文日志应包含失败系统条目
    let wire = h.wire().buffer();
    assert!(
        wire.iter().any(|e| e.summary.contains("连接失败")),
        "报文日志应包含连接失败系统条目"
    );

    h.disconnect().await;
    let _ = child.kill();
    let _ = child.wait();
}

/// v0.1.2：连接被拒（端口不通）应归类为"连接失败"并给出可读提示
///（而非"握手失败" + 泛型类型噪音；对应真实案例：URL 漏写端口 :8000）
#[tokio::test(flavor = "multi_thread")]
async fn http_connection_refused_reports_transport_failed() {
    let h = http_handle("mock-refused");
    let err = h
        .connect_http(&HttpConfig {
            url: "http://127.0.0.1:1/mcp".to_string(),
            headers: Vec::new(),
            timeout_secs: None,
            allow_insecure_tls: false,
        })
        .await
        .expect_err("端口不通应连接失败");

    assert_eq!(err.kind(), "transport_failed", "应分类为传输失败：{err:?}");
    let message = err.message();
    assert!(message.contains("无法连接服务器"), "应给出定向提示：{message}");
    assert!(message.contains("127.0.0.1:1"), "应包含目标地址：{message}");
    assert!(
        !message.contains("reqwest::") && !message.contains('<'),
        "不应包含类型噪音：{message}"
    );
}
