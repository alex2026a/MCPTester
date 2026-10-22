//! 期望判定与断言求值（纯函数）。

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::mcp::error::CallError;
use crate::mcp::models::{CallMethod, CallRecord, CallStatus};

use super::models::{
    AssertionKind, AssertionResult, AssertionSeverity, FailureKind, TestAssertion, TestExpect,
    TestOutcome, TestRunCaseStatus,
};
use super::schema;

/// 实际结果分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActualOutcome {
    Success,
    ToolError,
    RpcError,
    Timeout,
    TransportError,
    Canceled,
}

/// 断言求值结果。
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub status: TestRunCaseStatus,
    pub failure_kind: Option<FailureKind>,
    pub assertions: Vec<AssertionResult>,
    pub message: Option<String>,
}

/// 判定期望 + 断言。
pub fn evaluate(
    expect: &TestExpect,
    assertions: &[TestAssertion],
    record: &CallRecord,
    output_schema: Option<&Value>,
) -> Evaluation {
    let actual = classify(record);
    if matches!(actual, ActualOutcome::Timeout) {
        return Evaluation {
            status: TestRunCaseStatus::Error,
            failure_kind: Some(FailureKind::Timeout),
            assertions: Vec::new(),
            message: Some("调用超时".to_string()),
        };
    }
    if matches!(actual, ActualOutcome::TransportError) {
        return Evaluation {
            status: TestRunCaseStatus::Error,
            failure_kind: Some(FailureKind::TransportError),
            assertions: Vec::new(),
            message: Some("传输关闭或服务器断线".to_string()),
        };
    }
    if matches!(actual, ActualOutcome::Canceled) {
        return Evaluation {
            status: TestRunCaseStatus::Canceled,
            failure_kind: None,
            assertions: Vec::new(),
            message: Some("用例已取消".to_string()),
        };
    }

    let mut expectation_failure = expectation_failure(expect, &actual, record);
    let mut results = Vec::new();
    let mut has_error_failure = false;
    let mut has_warning = false;

    // 负向用例实际成功时，mcp_shape/output_schema 跳过，由 expect 决定 warning/failed。
    let skip_shape = expect.outcome == TestOutcome::Rejected
        && matches!(actual, ActualOutcome::Success);

    for assertion in assertions.iter().filter(|item| item.enabled) {
        let result = evaluate_assertion(assertion, record, &actual, output_schema, skip_shape);
        if !result.passed {
            match result.severity {
                AssertionSeverity::Error => has_error_failure = true,
                AssertionSeverity::Warning => has_warning = true,
            }
        }
        results.push(result);
    }

    if expectation_failure.is_none() {
        if has_error_failure {
            expectation_failure = Some((
                TestRunCaseStatus::Failed,
                FailureKind::AssertionFailed,
                "断言未通过".to_string(),
            ));
        } else if has_warning {
            expectation_failure = Some((
                TestRunCaseStatus::Warning,
                FailureKind::AssertionFailed,
                "警告级断言未通过".to_string(),
            ));
        } else if expect.outcome == TestOutcome::Any
            && !assertions.iter().any(|item| item.enabled)
        {
            expectation_failure = Some((
                TestRunCaseStatus::Warning,
                FailureKind::AssertionFailed,
                "用例没有启用断言".to_string(),
            ));
        }
    }

    match expectation_failure {
        Some((status, kind, message)) => Evaluation {
            status,
            failure_kind: Some(kind),
            assertions: results,
            message: Some(message),
        },
        None => Evaluation {
            status: TestRunCaseStatus::Passed,
            failure_kind: None,
            assertions: results,
            message: None,
        },
    }
}

fn expectation_failure(
    expect: &TestExpect,
    actual: &ActualOutcome,
    record: &CallRecord,
) -> Option<(TestRunCaseStatus, FailureKind, String)> {
    match expect.outcome {
        TestOutcome::Any => None,
        TestOutcome::Success => match actual {
            ActualOutcome::Success => None,
            ActualOutcome::ToolError => Some((
                TestRunCaseStatus::Failed,
                FailureKind::ToolErrorUnexpected,
                "期望成功，实际返回工具错误".to_string(),
            )),
            ActualOutcome::RpcError => Some((
                TestRunCaseStatus::Failed,
                FailureKind::ProtocolError,
                "期望成功，实际返回 JSON-RPC 错误".to_string(),
            )),
            _ => Some((
                TestRunCaseStatus::Error,
                FailureKind::Unknown,
                "非预期执行状态".to_string(),
            )),
        },
        TestOutcome::ToolError => match actual {
            ActualOutcome::ToolError => None,
            ActualOutcome::Success => Some((
                TestRunCaseStatus::Failed,
                FailureKind::AssertionFailed,
                "期望工具错误，实际成功".to_string(),
            )),
            ActualOutcome::RpcError => Some((
                TestRunCaseStatus::Failed,
                FailureKind::ProtocolError,
                "期望工具错误，实际为 JSON-RPC 错误".to_string(),
            )),
            _ => Some((
                TestRunCaseStatus::Error,
                FailureKind::Unknown,
                "非预期执行状态".to_string(),
            )),
        },
        TestOutcome::RpcError => match actual {
            ActualOutcome::RpcError => {
                if matches_rpc(expect, record) {
                    None
                } else {
                    Some((
                        TestRunCaseStatus::Failed,
                        FailureKind::AssertionFailed,
                        "JSON-RPC 错误码/消息不符合期望".to_string(),
                    ))
                }
            }
            ActualOutcome::ToolError => Some((
                TestRunCaseStatus::Failed,
                FailureKind::ToolErrorUnexpected,
                "期望 JSON-RPC 错误，实际为工具错误".to_string(),
            )),
            ActualOutcome::Success => Some((
                TestRunCaseStatus::Failed,
                FailureKind::AssertionFailed,
                "期望 JSON-RPC 错误，实际成功".to_string(),
            )),
            _ => Some((
                TestRunCaseStatus::Error,
                FailureKind::Unknown,
                "非预期执行状态".to_string(),
            )),
        },
        TestOutcome::Rejected => match actual {
            ActualOutcome::ToolError | ActualOutcome::RpcError => None,
            ActualOutcome::Success => {
                if expect.accept_as_warning {
                    Some((
                        TestRunCaseStatus::Warning,
                        FailureKind::InvalidInputAccepted,
                        "负向用例被服务端接受（警告）".to_string(),
                    ))
                } else {
                    Some((
                        TestRunCaseStatus::Failed,
                        FailureKind::InvalidInputAccepted,
                        "负向用例被服务端接受".to_string(),
                    ))
                }
            }
            _ => Some((
                TestRunCaseStatus::Error,
                FailureKind::Unknown,
                "非预期执行状态".to_string(),
            )),
        },
    }
}

fn matches_rpc(expect: &TestExpect, record: &CallRecord) -> bool {
    let Some(CallError::RpcError { code, message, .. }) = &record.error else {
        return false;
    };
    if *code == 0 {
        // code=0 是工具错误的合成码，不能当作协议错误码匹配。
        return false;
    }
    if let Some(expected) = expect.rpc_code
        && *code != expected
    {
        return false;
    }
    if let Some(pattern) = &expect.message_regex {
        match Regex::new(pattern) {
            Ok(regex) if regex.is_match(message) => {}
            Ok(_) => return false,
            Err(_) => return false,
        }
    }
    true
}

fn classify(record: &CallRecord) -> ActualOutcome {
    if record.status == CallStatus::Canceled {
        return ActualOutcome::Canceled;
    }
    match &record.error {
        Some(CallError::Timeout { .. }) => ActualOutcome::Timeout,
        Some(CallError::TransportClosed { .. }) => ActualOutcome::TransportError,
        Some(CallError::RpcError { code, .. }) => {
            if *code == 0 && result_is_error(record.result.as_ref()) {
                ActualOutcome::ToolError
            } else {
                ActualOutcome::RpcError
            }
        }
        Some(_) => ActualOutcome::RpcError,
        None => {
            if result_is_error(record.result.as_ref()) {
                ActualOutcome::ToolError
            } else {
                ActualOutcome::Success
            }
        }
    }
}

fn result_is_error(result: Option<&Value>) -> bool {
    result
        .and_then(Value::as_object)
        .map(|object| {
            object
                .get("isError")
                .or_else(|| object.get("is_error"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn evaluate_assertion(
    assertion: &TestAssertion,
    record: &CallRecord,
    actual: &ActualOutcome,
    output_schema: Option<&Value>,
    skip_shape: bool,
) -> AssertionResult {
    let mut result = AssertionResult {
        assertion_id: assertion.id.clone(),
        kind: assertion.kind.clone(),
        severity: assertion.severity.clone(),
        passed: true,
        expected: assertion.expected.clone(),
        actual: None,
        message: assertion.message.clone(),
    };

    match assertion.kind {
        AssertionKind::McpShape => {
            if skip_shape {
                result.message = Some("负向用例被接受，跳过结果形状断言".to_string());
                return result;
            }
            match mcp_shape(record, actual) {
                Ok(actual_value) => {
                    result.actual = actual_value;
                }
                Err(message) => {
                    result.passed = false;
                    result.message = Some(message);
                }
            }
        }
        AssertionKind::OutputSchema => {
            if matches!(actual, ActualOutcome::ToolError | ActualOutcome::RpcError) {
                result.message = Some("非成功结果，跳过 outputSchema".to_string());
                return result;
            }
            let Some(structured) = structured_content(record.result.as_ref()) else {
                result.passed = assertion.severity == AssertionSeverity::Warning;
                result.message = Some("结果没有 structuredContent".to_string());
                return result;
            };
            let schema = assertion
                .schema
                .as_ref()
                .or(output_schema);
            let Some(schema) = schema else {
                result.message = Some("工具未声明 outputSchema，跳过".to_string());
                return result;
            };
            let report = schema::validate(schema, structured);
            result.actual = Some(structured.clone());
            if !report.valid {
                result.passed = false;
                result.message = Some(report.errors.join("; "));
            }
        }
        AssertionKind::JsonPath => {
            let root = target_value(record, assertion.target.as_deref());
            let Some(root) = root else {
                result.passed = false;
                result.message = Some("没有可断言的结果".to_string());
                return result;
            };
            match eval_json_path(root, assertion) {
                Ok((actual_value, passed, message)) => {
                    result.actual = Some(actual_value);
                    result.passed = passed;
                    if !passed {
                        result.message = Some(message);
                    }
                }
                Err(message) => {
                    result.passed = false;
                    result.message = Some(message);
                }
            }
        }
        AssertionKind::Content => {
            let text = content_text(record.result.as_ref());
            let passed = match assertion.op.as_deref().unwrap_or("contains") {
                "contains" => text.contains(
                    assertion
                        .expected
                        .as_ref()
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                "regex" => assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .and_then(|pattern| Regex::new(pattern).ok())
                    .map(|regex| regex.is_match(&text))
                    .unwrap_or(false),
                _ => false,
            };
            result.actual = Some(Value::String(text));
            if !passed {
                result.passed = false;
                result.message = Some("文本内容不匹配".to_string());
            }
        }
        AssertionKind::Duration => {
            let limit = assertion.value_ms.unwrap_or(u64::MAX);
            result.actual = Some(json!(record.duration_ms));
            if record.duration_ms > limit {
                result.passed = false;
                result.message = Some(format!(
                    "耗时 {}ms 超过 {}ms",
                    record.duration_ms, limit
                ));
            }
        }
        AssertionKind::Snapshot => {
            let Some(expected) = &assertion.expected else {
                result.passed = false;
                result.message = Some("快照断言缺少 expected".to_string());
                return result;
            };
            let Some(actual_value) = record.result.as_ref() else {
                result.passed = false;
                result.message = Some("没有可比较的结果".to_string());
                return result;
            };
            let diffs = diff_values(expected, actual_value, "$", &assertion.ignore_paths, 200);
            result.actual = Some(actual_value.clone());
            if !diffs.is_empty() {
                result.passed = false;
                result.message = Some(diffs.join("; "));
            }
        }
    }
    result
}

fn mcp_shape(record: &CallRecord, actual: &ActualOutcome) -> Result<Option<Value>, String> {
    if matches!(actual, ActualOutcome::RpcError) {
        let Some(CallError::RpcError { code, message, .. }) = &record.error else {
            return Err("缺少 JSON-RPC 错误包络".to_string());
        };
        if *code == 0 {
            return Err("工具错误被误判为协议错误".to_string());
        }
        return Ok(Some(json!({ "rpc_error": { "code": code, "message": message } })));
    }
    let Some(result) = record.result.as_ref() else {
        return Err("结果为空".to_string());
    };
    let Some(object) = result.as_object() else {
        return Err("结果不是 JSON 对象".to_string());
    };
    match record.method {
        CallMethod::ToolsCall => {
            if let Some(is_error) = object.get("isError").or_else(|| object.get("is_error"))
                && !is_error.is_boolean()
            {
                return Err("isError 不是布尔值".to_string());
            }
            let Some(content) = object.get("content") else {
                if matches!(actual, ActualOutcome::ToolError) {
                    return Ok(Some(json!({"tool_error": true})));
                }
                return Err("缺少 content 数组".to_string());
            };
            let Some(items) = content.as_array() else {
                return Err("content 不是数组".to_string());
            };
            for item in items {
                let Some(item) = item.as_object() else {
                    return Err("content 项不是对象".to_string());
                };
                let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if !matches!(kind, "text" | "image" | "audio" | "resource" | "resource_link") {
                    return Err(format!("未知 content 类型 {kind}"));
                }
            }
            Ok(Some(json!({ "content_blocks": items.len() })))
        }
        CallMethod::ResourcesRead => {
            let Some(contents) = object.get("contents").and_then(Value::as_array) else {
                return Err("缺少 contents 数组".to_string());
            };
            for item in contents {
                let Some(item) = item.as_object() else {
                    return Err("contents 项不是对象".to_string());
                };
                if !item.get("uri").map(Value::is_string).unwrap_or(false) {
                    return Err("contents 项缺少 uri".to_string());
                }
            }
            Ok(Some(json!({ "resource_contents": contents.len() })))
        }
        CallMethod::PromptsGet => {
            let Some(messages) = object.get("messages").and_then(Value::as_array) else {
                return Err("缺少 messages 数组".to_string());
            };
            for item in messages {
                let Some(item) = item.as_object() else {
                    return Err("messages 项不是对象".to_string());
                };
                if !item.get("role").map(Value::is_string).unwrap_or(false) {
                    return Err("messages 项缺少 role".to_string());
                }
            }
            Ok(Some(json!({ "prompt_messages": messages.len() })))
        }
        CallMethod::Probe => Ok(None),
    }
}

fn structured_content(result: Option<&Value>) -> Option<&Value> {
    result
        .and_then(Value::as_object)
        .and_then(|object| {
            object
                .get("structuredContent")
                .or_else(|| object.get("structured_content"))
        })
}

fn target_value<'a>(record: &'a CallRecord, target: Option<&str>) -> Option<&'a Value> {
    match target.unwrap_or("result") {
        "structured_content" => structured_content(record.result.as_ref()),
        "content_text" => None,
        _ => record.result.as_ref(),
    }
}

fn content_text(result: Option<&Value>) -> String {
    let Some(result) = result else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    if let Some(contents) = result.get("contents").and_then(Value::as_array) {
        for item in contents {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    if let Some(messages) = result.get("messages").and_then(Value::as_array) {
        for item in messages {
            if let Some(text) = item
                .get("content")
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
            {
                parts.push(text.to_string());
            }
        }
    }
    parts.join("\n")
}

fn eval_json_path(
    root: &Value,
    assertion: &TestAssertion,
) -> Result<(Value, bool, String), String> {
    let path = assertion.path.as_deref().unwrap_or("$");
    let tokens = parse_path(path)?;
    let actual = extract(root, &tokens)?;
    let op = assertion.op.as_deref().unwrap_or("exists");
    let expected = assertion.expected.as_ref();
    let passed = match op {
        "exists" => actual.is_some(),
        "eq" => expected
            .map(|expected| actual.as_ref() == Some(expected))
            .unwrap_or(false),
        "neq" => expected
            .map(|expected| actual.as_ref() != Some(expected))
            .unwrap_or(false),
        "contains" => match (actual.as_ref(), expected) {
            (Some(Value::String(text)), Some(Value::String(needle))) => text.contains(needle),
            (Some(Value::Array(items)), Some(expected)) => items.contains(expected),
            _ => false,
        },
        "regex" => match (actual.as_ref(), expected.and_then(Value::as_str)) {
            (Some(Value::String(text)), Some(pattern)) => Regex::new(pattern)
                .map(|regex| regex.is_match(text))
                .unwrap_or(false),
            _ => false,
        },
        "type" => match (actual.as_ref(), expected.and_then(Value::as_str)) {
            (Some(value), Some(kind)) => type_name(value) == kind,
            _ => false,
        },
        "length" => {
            let length = match actual.as_ref() {
                Some(Value::Array(items)) => Some(items.len() as u64),
                Some(Value::String(text)) => Some(text.chars().count() as u64),
                Some(Value::Object(map)) => Some(map.len() as u64),
                _ => None,
            };
            expected
                .and_then(Value::as_u64)
                .map(|expected| length == Some(expected))
                .unwrap_or(false)
        }
        "gt" | "gte" | "lt" | "lte" => {
            let left = actual.as_ref().and_then(Value::as_f64);
            let right = expected.and_then(Value::as_f64);
            match (left, right) {
                (Some(left), Some(right)) => match op {
                    "gt" => left > right,
                    "gte" => left >= right,
                    "lt" => left < right,
                    _ => left <= right,
                },
                _ => false,
            }
        }
        "in" => match expected {
            Some(Value::Array(items)) => actual
                .as_ref()
                .map(|value| items.contains(value))
                .unwrap_or(false),
            _ => false,
        },
        _ => false,
    };
    Ok((
        actual.unwrap_or(Value::Null),
        passed,
        format!("JSONPath {path} 断言失败（op={op}）"),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathToken {
    Key(String),
    Index(i64),
    Wildcard,
}

fn parse_path(path: &str) -> Result<Vec<PathToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = path.chars().peekable();
    if chars.next() != Some('$') {
        return Err("JSONPath 必须以 $ 开头".to_string());
    }
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                let mut key = String::new();
                while let Some(next) = chars.peek() {
                    if *next == '.' || *next == '[' {
                        break;
                    }
                    key.push(chars.next().unwrap());
                }
                if key.is_empty() {
                    return Err("JSONPath 存在空键".to_string());
                }
                tokens.push(PathToken::Key(key));
            }
            '[' => {
                let mut content = String::new();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                    content.push(next);
                }
                if content == "*" {
                    tokens.push(PathToken::Wildcard);
                } else if let Ok(index) = content.parse::<i64>() {
                    tokens.push(PathToken::Index(index));
                } else {
                    tokens.push(PathToken::Key(
                        content.trim_matches(|ch| ch == '\'' || ch == '"').to_string(),
                    ));
                }
            }
            _ => return Err(format!("JSONPath 含非法字符 {ch}")),
        }
    }
    Ok(tokens)
}

fn extract(value: &Value, tokens: &[PathToken]) -> Result<Option<Value>, String> {
    let mut current = value.clone();
    for token in tokens {
        match token {
            PathToken::Key(key) => {
                let Some(next) = current.get(key) else {
                    return Ok(None);
                };
                current = next.clone();
            }
            PathToken::Index(index) => {
                let Some(array) = current.as_array() else {
                    return Ok(None);
                };
                let resolved = if *index < 0 {
                    array.len() as i64 + index
                } else {
                    *index
                };
                let Some(next) = array.get(resolved.max(0) as usize) else {
                    return Ok(None);
                };
                current = next.clone();
            }
            PathToken::Wildcard => {
                let Some(array) = current.as_array() else {
                    return Ok(None);
                };
                current = Value::Array(array.clone());
            }
        }
    }
    Ok(Some(current))
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.as_f64().map(|v| v.fract() == 0.0).unwrap_or(false) {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn diff_values(
    expected: &Value,
    actual: &Value,
    path: &str,
    ignore_paths: &[String],
    limit: usize,
) -> Vec<String> {
    let mut diffs = Vec::new();
    diff_at(expected, actual, path, ignore_paths, limit, &mut diffs);
    diffs
}

fn diff_at(
    expected: &Value,
    actual: &Value,
    path: &str,
    ignore_paths: &[String],
    limit: usize,
    diffs: &mut Vec<String>,
) {
    if diffs.len() >= limit || is_ignored(path, ignore_paths) {
        return;
    }
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let next = format!("{path}.{key}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        diff_at(left, right, &next, ignore_paths, limit, diffs)
                    }
                    (Some(_), None) => diffs.push(format!("{next}: 实际缺失")),
                    (None, Some(_)) => diffs.push(format!("{next}: 实际多出")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                diffs.push(format!(
                    "{path}: 数组长度 {} != {}",
                    left.len(),
                    right.len()
                ));
            }
            for (index, (left, right)) in left.iter().zip(right.iter()).enumerate() {
                diff_at(
                    left,
                    right,
                    &format!("{path}[{index}]"),
                    ignore_paths,
                    limit,
                    diffs,
                );
            }
        }
        _ if expected != actual => diffs.push(format!(
            "{path}: 期望 {}，实际 {}",
            compact(expected),
            compact(actual)
        )),
        _ => {}
    }
}

fn is_ignored(path: &str, ignore_paths: &[String]) -> bool {
    ignore_paths.iter().any(|ignore| {
        path == ignore
            || path.starts_with(&format!("{ignore}."))
            || path.starts_with(&format!("{ignore}["))
    })
}

fn compact(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    if text.chars().count() > 120 {
        format!("{}…", text.chars().take(120).collect::<String>())
    } else {
        text
    }
}

/// 提取结构化内容中的字段（供测试/报告复用）。
#[allow(dead_code)]
pub fn structured_field(result: &Value, key: &str) -> Option<Value> {
    let structured = structured_content(Some(result))?;
    let map: &Map<String, Value> = structured.as_object()?;
    map.get(key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{CallOrigin, TransportKind};
    use chrono::Utc;
    use uuid::Uuid;

    fn record(
        status: CallStatus,
        error: Option<CallError>,
        result: Option<Value>,
        method: CallMethod,
    ) -> CallRecord {
        CallRecord {
            id: Uuid::new_v4().to_string(),
            server_id: "s1".to_string(),
            server_name: "mock".to_string(),
            method,
            target: "echo".to_string(),
            params: json!({}),
            result,
            error,
            status,
            duration_ms: 12,
            protocol_version: "2026-07-28".to_string(),
            transport: TransportKind::Stdio,
            truncated: false,
            size_bytes: 0,
            origin: CallOrigin::Test,
            created_at: Utc::now(),
        }
    }

    fn assertion(kind: AssertionKind) -> TestAssertion {
        TestAssertion {
            id: "a1".to_string(),
            kind,
            severity: AssertionSeverity::Error,
            enabled: true,
            target: None,
            path: None,
            op: None,
            expected: None,
            value_ms: None,
            message: None,
            ignore_paths: Vec::new(),
            schema: None,
            schema_ref: None,
        }
    }

    fn expect(outcome: TestOutcome) -> TestExpect {
        TestExpect {
            outcome,
            ..Default::default()
        }
    }

    #[test]
    fn success_and_tool_error_matrix() {
        let ok = record(
            CallStatus::Ok,
            None,
            Some(json!({"content": [{"type": "text", "text": "ok"}], "isError": false})),
            CallMethod::ToolsCall,
        );
        let eval = evaluate(&expect(TestOutcome::Success), &[], &ok, None);
        assert_eq!(eval.status, TestRunCaseStatus::Passed);

        let tool_error = record(
            CallStatus::Error,
            Some(CallError::RpcError {
                code: 0,
                message: "工具返回 isError=true".to_string(),
                data: None,
            }),
            Some(json!({"content": [], "isError": true})),
            CallMethod::ToolsCall,
        );
        assert_eq!(
            evaluate(&expect(TestOutcome::ToolError), &[], &tool_error, None).status,
            TestRunCaseStatus::Passed
        );
        assert_eq!(
            evaluate(&expect(TestOutcome::Success), &[], &tool_error, None).status,
            TestRunCaseStatus::Failed
        );
        assert_eq!(
            evaluate(&expect(TestOutcome::Rejected), &[], &tool_error, None).status,
            TestRunCaseStatus::Passed
        );
    }

    #[test]
    fn rejected_success_is_warning_or_failed() {
        let ok = record(
            CallStatus::Ok,
            None,
            Some(json!({"content": []})),
            CallMethod::ToolsCall,
        );
        let mut expect = expect(TestOutcome::Rejected);
        expect.accept_as_warning = true;
        assert_eq!(
            evaluate(&expect, &[], &ok, None).status,
            TestRunCaseStatus::Warning
        );
        expect.accept_as_warning = false;
        assert_eq!(
            evaluate(&expect, &[], &ok, None).status,
            TestRunCaseStatus::Failed
        );
    }

    #[test]
    fn code_zero_never_matches_rpc_code() {
        let tool_error = record(
            CallStatus::Error,
            Some(CallError::RpcError {
                code: 0,
                message: "工具错误".to_string(),
                data: None,
            }),
            Some(json!({"content": [], "isError": true})),
            CallMethod::ToolsCall,
        );
        let mut expect = expect(TestOutcome::RpcError);
        expect.rpc_code = Some(0);
        assert_eq!(
            evaluate(&expect, &[], &tool_error, None).status,
            TestRunCaseStatus::Failed
        );
    }

    #[test]
    fn json_path_and_snapshot() {
        let record = record(
            CallStatus::Ok,
            None,
            Some(json!({"structuredContent": {"items": [{"id": 1}], "ok": true}})),
            CallMethod::ToolsCall,
        );
        let mut path = assertion(AssertionKind::JsonPath);
        path.path = Some("$.structuredContent.items[0].id".to_string());
        path.op = Some("eq".to_string());
        path.expected = Some(json!(1));
        let eval = evaluate(&expect(TestOutcome::Any), &[path], &record, None);
        assert_eq!(eval.status, TestRunCaseStatus::Passed);

        let mut snapshot = assertion(AssertionKind::Snapshot);
        snapshot.expected = Some(json!({"structuredContent": {"items": [{"id": 2}], "ok": true}}));
        let eval = evaluate(&expect(TestOutcome::Any), &[snapshot], &record, None);
        assert_eq!(eval.status, TestRunCaseStatus::Failed);
    }

    #[test]
    fn mcp_shape_requires_content() {
        let record = record(
            CallStatus::Ok,
            None,
            Some(json!({"structuredContent": {"ok": true}})),
            CallMethod::ToolsCall,
        );
        let eval = evaluate(
            &expect(TestOutcome::Success),
            &[assertion(AssertionKind::McpShape)],
            &record,
            None,
        );
        assert_eq!(eval.status, TestRunCaseStatus::Failed);
    }
}
