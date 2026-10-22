//! M2 集成测试：能力快照 → 确定性生成 → 断言求值。

#![cfg(feature = "mock-server")]

use std::sync::Arc;

use mcptester_lib::mcp::connection::{CallOptions, ConnectionHandle};
use mcptester_lib::mcp::models::{
    CallOrigin, RuntimeSettings, ShellWrap, StdioConfig, TransportKind,
};
use mcptester_lib::testing::assertions;
use mcptester_lib::testing::generator;
use mcptester_lib::testing::models::{GenerationOptions, TestMethod, TestOutcome};

const MOCK_BIN: &str = env!("CARGO_BIN_EXE_mock-mcp-server");

fn stdio_cfg(args: &[&str]) -> StdioConfig {
    StdioConfig {
        command: MOCK_BIN.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: Vec::new(),
        cwd: None,
        shell_wrap: ShellWrap::Never,
    }
}

fn handle(id: &str) -> Arc<ConnectionHandle> {
    ConnectionHandle::new(id, id, None, TransportKind::Stdio, RuntimeSettings::default())
}

#[tokio::test(flavor = "multi_thread")]
async fn generate_preview_is_deterministic_and_assertion_passes() {
    let h = handle("mock-generate");
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "both"]))
        .await
        .expect("连接失败");
    let snapshot = h.capabilities(true).await.expect("枚举失败");
    let options = GenerationOptions::default();

    let first = generator::generate(&snapshot, &[], &options);
    let second = generator::generate(&snapshot, &[], &options);
    assert_eq!(first.capability_hash.len(), 32);
    assert!(first.stats.positive >= 1);
    assert!(first.stats.boundary >= 1 || first.stats.negative >= 1);
    assert_eq!(
        first
            .cases
            .iter()
            .map(|case| (case.name.clone(), case.arguments.clone()))
            .collect::<Vec<_>>(),
        second
            .cases
            .iter()
            .map(|case| (case.name.clone(), case.arguments.clone()))
            .collect::<Vec<_>>(),
        "同输入同种子应生成相同用例"
    );
    assert!(
        first
            .cases
            .iter()
            .any(|case| case.method == TestMethod::ToolsCall && case.target == "echo"),
        "应生成 echo 用例"
    );

    let positive = first
        .cases
        .iter()
        .find(|case| {
            case.method == TestMethod::ToolsCall
                && case.target == "echo"
                && case.expect.outcome == TestOutcome::Success
        })
        .expect("缺少 echo 正向用例");
    let record = h
        .call_tool("echo", positive.arguments.clone())
        .await;
    let output_schema = snapshot
        .tools
        .iter()
        .find(|tool| tool.name == "echo")
        .and_then(|tool| tool.output_schema.as_ref());
    let evaluation = assertions::evaluate(
        &positive.expect,
        &positive.assertions,
        &record,
        output_schema,
    );
    assert_eq!(
        evaluation.status,
        mcptester_lib::testing::models::TestRunCaseStatus::Passed,
        "正向用例应通过：{:?}",
        evaluation.message
    );

    h.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_options_keep_call_id_timeout_and_origin() {
    let h = handle("mock-call-options");
    h.connect_stdio(&stdio_cfg(&["--transport", "stdio", "--era", "both"]))
        .await
        .expect("连接失败");

    let record = h
        .call_tool_with_options(
            "bounded",
            serde_json::json!({
                "name": "ok",
                "count": 2,
                "tags": ["a"],
                "mode": "a"
            }),
            CallOptions {
                call_id: Some("case-call-1".to_string()),
                timeout_secs: Some(5),
                origin: CallOrigin::Test,
            },
        )
        .await;
    assert_eq!(record.id, "case-call-1");
    assert_eq!(record.origin, CallOrigin::Test);
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Ok);

    let invalid = h
        .call_tool_with_options(
            "bounded",
            serde_json::json!({ "name": "x", "count": 9, "tags": [] }),
            CallOptions {
                call_id: Some("case-call-2".to_string()),
                timeout_secs: Some(5),
                origin: CallOrigin::Test,
            },
        )
        .await;
    assert_eq!(
        invalid.status,
        mcptester_lib::mcp::models::CallStatus::Error,
        "非法参数应返回错误"
    );
    assert!(
        serde_json::to_string(&invalid.result)
            .unwrap()
            .contains("isError")
    );

    let prompt = h
        .get_prompt_with_options(
            "greet",
            serde_json::json!({"name": "MCP"}),
            CallOptions {
                call_id: Some("case-call-3".to_string()),
                timeout_secs: Some(5),
                origin: CallOrigin::Test,
            },
        )
        .await;
    assert_eq!(prompt.id, "case-call-3");
    assert_eq!(prompt.status, mcptester_lib::mcp::models::CallStatus::Ok);

    h.disconnect().await;
}
