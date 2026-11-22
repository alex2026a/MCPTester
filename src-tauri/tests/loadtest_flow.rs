//! v0.7.0 P0 压测引擎集成测试（mock server）。
//!
//! 运行：`cargo test --features mock-server --test loadtest_flow`

#![cfg(feature = "mock-server")]

use std::time::Duration;

use mcptester_lib::loadtest::engine::{LoadContext, run};
use mcptester_lib::loadtest::models::{
    LoadCaseSnapshot, LoadOperation, LoadProfile, LoadRunControl, LoadRunStatus, LoadSessionMode,
    LoadStage,
};
use mcptester_lib::loadtest::session::LoadSessionFactory;
use mcptester_lib::mcp::models::{
    HttpConfig, KeyValue, ServerConfig, ShellWrap, StdioConfig, TransportKind,
};
use mcptester_lib::testing::models::{TestMethod, TestRisk};
use serde_json::json;
use serial_test::serial;

const MOCK_BIN: &str = env!("CARGO_BIN_EXE_mock-mcp-server");

fn stdio_config(args: &[&str]) -> ServerConfig {
    ServerConfig {
        id: "load-stdio".to_string(),
        name: "mock-load".to_string(),
        transport: TransportKind::Stdio,
        stdio: Some(StdioConfig {
            command: MOCK_BIN.to_string(),
            args: args.iter().map(|item| item.to_string()).collect(),
            env: Vec::<KeyValue>::new(),
            cwd: None,
            shell_wrap: ShellWrap::Never,
        }),
        http: None,
        protocol_version_pin: None,
        enabled: true,
        tags: Vec::new(),
        origin: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_connected_at: None,
    }
}

fn http_config(url: String) -> ServerConfig {
    ServerConfig {
        id: "load-http".to_string(),
        name: "mock-load-http".to_string(),
        transport: TransportKind::StreamableHttp,
        stdio: None,
        http: Some(HttpConfig {
            url,
            headers: Vec::new(),
            timeout_secs: None,
            allow_insecure_tls: false,
            oauth: None,
        }),
        protocol_version_pin: None,
        enabled: true,
        tags: Vec::new(),
        origin: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_connected_at: None,
    }
}

fn sse_config(url: String) -> ServerConfig {
    let mut config = http_config(url);
    config.transport = TransportKind::Sse;
    config.id = "load-sse".to_string();
    config.name = "mock-load-sse".to_string();
    config
}

fn echo_operation() -> LoadCaseSnapshot {
    LoadCaseSnapshot {
        case_id: "c1".to_string(),
        name: "echo".to_string(),
        method: TestMethod::ToolsCall,
        target: "echo".to_string(),
        arguments: json!({ "text": "load" }),
        risk: TestRisk::ReadOnly,
        weight: 1,
        timeout_secs: None,
    }
}

fn add_operation() -> LoadCaseSnapshot {
    LoadCaseSnapshot {
        case_id: "c2".to_string(),
        name: "add".to_string(),
        method: TestMethod::ToolsCall,
        target: "add".to_string(),
        arguments: json!({ "a": 1, "b": 2 }),
        risk: TestRisk::ReadOnly,
        weight: 1,
        timeout_secs: None,
    }
}

fn profile(mode: LoadSessionMode, vus: u32, secs: u64, think_ms: u64) -> LoadProfile {
    LoadProfile {
        name: "load-test".to_string(),
        session_mode: mode,
        stages: vec![LoadStage {
            duration_secs: secs,
            target_vus: vus,
        }],
        think_time_min_ms: think_ms,
        think_time_max_ms: think_ms,
        timeout_secs: 5,
        operations: vec![
            LoadOperation {
                case_id: "c1".to_string(),
                weight: 1,
            },
            LoadOperation {
                case_id: "c2".to_string(),
                weight: 1,
            },
        ],
        allow_write: false,
        allow_unknown: false,
        abort_error_rate: Some(0.5),
        abort_p99_ms: None,
        seed: 7,
    }
}

async fn run_load(
    config: ServerConfig,
    profile: LoadProfile,
    operations: Vec<LoadCaseSnapshot>,
) -> mcptester_lib::loadtest::engine::LoadOutcome {
    let control = LoadRunControl::new("run-1".to_string(), config.id.clone());
    run(
        LoadContext {
            run_id: "run-1".to_string(),
            server_id: config.id.clone(),
            server_name: config.name.clone(),
            profile,
            operations,
            config,
            oauth: None,
            event_sink: None,
        },
        control,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn stdio_per_vu_runs_and_cleans_up() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let outcome = run_load(
        config,
        profile(LoadSessionMode::PerVu, 2, 5, 0),
        vec![echo_operation(), add_operation()],
    )
    .await;
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0, "应至少完成一次调用");
    assert!(
        outcome.summary.success_rate >= 0.99,
        "mock echo/add 应全部成功：{:?}",
        outcome.summary
    );
    assert!(!outcome.series.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn stdio_shared_runs_without_deadlock() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let outcome = run_load(
        config,
        profile(LoadSessionMode::Shared, 3, 2, 0),
        vec![echo_operation(), add_operation()],
    )
    .await;
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0);
    assert!(outcome.summary.success_rate >= 0.99);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[ignore = "重型 smoke：8 个 stdio 子进程，默认门禁不跑"]
async fn stdio_8_vus_smoke() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let outcome = run_load(
        config,
        profile(LoadSessionMode::PerVu, 8, 3, 0),
        vec![echo_operation(), add_operation()],
    )
    .await;
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0);
    assert!(outcome.summary.success_rate >= 0.95);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn shared_20_vus_smoke() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let outcome = run_load(
        config,
        profile(LoadSessionMode::Shared, 20, 2, 0),
        vec![echo_operation(), add_operation()],
    )
    .await;
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0);
    assert!(outcome.summary.success_rate >= 0.95);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn http_per_vu_runs() {
    let port = 18400 + (std::process::id() % 200) as u16;
    let mut child = std::process::Command::new(MOCK_BIN)
        .args([
            "--transport",
            "http",
            "--era",
            "both",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("mock HTTP 进程启动失败");
    let url = format!("http://127.0.0.1:{port}/mcp");
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
    assert!(ready, "mock HTTP 服务未就绪");

    let outcome = run_load(
        http_config(url),
        profile(LoadSessionMode::PerVu, 2, 2, 0),
        vec![echo_operation()],
    )
    .await;
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0);
    assert!(outcome.summary.success_rate >= 0.99);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn sse_single_vu_smoke() {
    let port = 18600 + (std::process::id() % 200) as u16;
    let mut child = std::process::Command::new(MOCK_BIN)
        .args([
            "--transport",
            "sse",
            "--era",
            "both",
            "--port",
            &port.to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("mock SSE 进程启动失败");
    let url = format!("http://127.0.0.1:{port}/sse");
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
    assert!(ready, "mock SSE 服务未就绪");

    let outcome = run_load(
        sse_config(url),
        profile(LoadSessionMode::PerVu, 1, 2, 0),
        vec![echo_operation()],
    )
    .await;
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(outcome.status, LoadRunStatus::Completed);
    assert!(outcome.summary.total > 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn cancel_with_long_think_time_returns_quickly() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let control = LoadRunControl::new("run-cancel".to_string(), config.id.clone());
    let run_control = control.clone();
    let task = tokio::spawn(run(
        LoadContext {
            run_id: "run-cancel".to_string(),
            server_id: config.id.clone(),
            server_name: config.name.clone(),
            profile: profile(LoadSessionMode::PerVu, 1, 60, 60_000),
            operations: vec![echo_operation()],
            config,
            oauth: None,
            event_sink: None,
        },
        run_control,
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;
    control.request_cancel("user");
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("取消应在 10s 内返回")
        .expect("压测任务不应 panic");
    assert_eq!(outcome.status, LoadRunStatus::Canceled);
    assert_eq!(outcome.stop_reason.as_deref(), Some("user"));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn quiet_session_does_not_capture_wire() {
    let config = stdio_config(&["--transport", "stdio", "--era", "both"]);
    let factory = LoadSessionFactory::new(config, &profile(LoadSessionMode::PerVu, 1, 1, 0), None);
    let handle = factory.connect(0).await.expect("连接失败");
    assert!(!handle.wire().enabled());
    let record = handle
        .call_tool("echo", json!({ "text": "quiet" }))
        .await;
    assert_eq!(
        record.status,
        mcptester_lib::mcp::models::CallStatus::Ok
    );
    assert_eq!(handle.wire().pending_len(), 0);
    assert_eq!(handle.wire().capture_count(), 0);
    assert!(handle.wire().buffer().is_empty());
    handle.disconnect().await;
}

