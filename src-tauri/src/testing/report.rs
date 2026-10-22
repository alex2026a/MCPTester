//! 报告构建与 HTML / JSON / JUnit / Markdown 导出。

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use super::models::{
    Coverage, Evidence, FailureKind, TestRun, TestRunCaseDetail, TestRunCaseStatus,
};

/// 报告数据（JSON/HTML/JUnit 共用）。
#[derive(Serialize)]
pub struct ReportData {
    pub report_schema_version: u32,
    pub generated_at: DateTime<Utc>,
    pub run: TestRun,
    pub coverage: Coverage,
    pub cases: Vec<TestRunCaseDetail>,
}

pub fn build(run: TestRun, coverage: Coverage, mut cases: Vec<TestRunCaseDetail>) -> ReportData {
    cases.sort_by(|left, right| {
        left.summary
            .order_index
            .cmp(&right.summary.order_index)
            .then(left.summary.id.cmp(&right.summary.id))
    });
    ReportData {
        report_schema_version: 1,
        generated_at: Utc::now(),
        run,
        coverage,
        cases,
    }
}

/// 生成 JSON 报告。
pub fn to_json(report: &ReportData, include_sensitive: bool) -> String {
    let value = report_value(report, include_sensitive);
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

/// 生成 HTML 报告（单文件、内嵌 CSS）。
pub fn to_html(report: &ReportData, include_sensitive: bool) -> String {
    let run = &report.run;
    let summary = &run.summary;
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>MCP Tester 测试报告</title>");
    html.push_str("<style>");
    html.push_str("body{font-family:system-ui,'Segoe UI',sans-serif;margin:0;background:#f8fafc;color:#0f172a}");
    html.push_str(".wrap{max-width:1200px;margin:0 auto;padding:24px}h1{font-size:22px}h2{font-size:16px;margin-top:28px}");
    html.push_str(".cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px}");
    html.push_str(".card{background:#fff;border:1px solid #e2e8f0;border-radius:8px;padding:12px}");
    html.push_str(".num{font-size:24px;font-weight:600}.muted{color:#64748b;font-size:12px}");
    html.push_str("table{width:100%;border-collapse:collapse;background:#fff;font-size:12px}");
    html.push_str("th,td{border-bottom:1px solid #e2e8f0;padding:8px;text-align:left;vertical-align:top}");
    html.push_str("th{background:#f1f5f9;color:#475569;font-weight:600}");
    html.push_str(".passed{color:#059669}.failed{color:#dc2626}.warning{color:#d97706}.skipped{color:#64748b}.error{color:#b91c1c}");
    html.push_str("pre{white-space:pre-wrap;word-break:break-all;background:#0f172a;color:#e2e8f0;padding:10px;border-radius:6px;font-size:11px;max-height:360px;overflow:auto}");
    html.push_str(".case{border:1px solid #e2e8f0;border-radius:8px;background:#fff;margin:12px 0;padding:12px}");
    html.push_str(".ok{color:#059669}.bad{color:#dc2626}.warn{color:#d97706}");
    html.push_str("</style></head><body><div class=\"wrap\">");
    html.push_str(&format!(
        "<h1>MCP Tester 测试报告 · {}</h1>",
        escape(&run.suite_name)
    ));
    html.push_str(&format!(
        "<div class=\"muted\">运行 {} · {} · {} · 开始 {} · 耗时 {}ms</div>",
        escape(&run.id),
        escape(&run.server_name),
        escape(&run.manifest.protocol_version),
        escape(&run.started_at.to_rfc3339()),
        summary.duration_ms
    ));
    html.push_str("<h2>概览</h2><div class=\"cards\">");
    for (label, value, class) in [
        ("总数", summary.total, ""),
        ("通过", summary.passed, "passed"),
        ("失败", summary.failed, "failed"),
        ("警告", summary.warning, "warning"),
        ("跳过", summary.skipped, "skipped"),
        ("错误", summary.error, "error"),
        ("取消", summary.canceled, "skipped"),
    ] {
        html.push_str(&format!(
            "<div class=\"card\"><div class=\"muted\">{label}</div><div class=\"num {class}\">{value}</div></div>"
        ));
    }
    html.push_str(&format!(
        "<div class=\"card\"><div class=\"muted\">有效通过率</div><div class=\"num\">{:.1}%</div></div>",
        summary.effective_pass_rate * 100.0
    ));
    html.push_str("</div>");

    html.push_str("<h2>覆盖率</h2><div class=\"cards\">");
    for (label, covered, total) in [
        ("Tools", report.coverage.tools_covered, report.coverage.tools_total),
        ("Resources", report.coverage.resources_covered, report.coverage.resources_total),
        ("Prompts", report.coverage.prompts_covered, report.coverage.prompts_total),
        (
            "必填参数",
            report.coverage.required_params_covered,
            report.coverage.required_params_total,
        ),
        (
            "约束",
            report.coverage.constraints_covered,
            report.coverage.constraints_total,
        ),
    ] {
        html.push_str(&format!(
            "<div class=\"card\"><div class=\"muted\">{label}</div><div class=\"num\">{covered}/{total}</div></div>"
        ));
    }
    html.push_str("</div>");

    html.push_str("<h2>运行清单</h2><table><tbody>");
    for (key, value) in [
        ("应用版本", run.manifest.app_version.clone()),
        ("服务器", run.manifest.server_name.clone()),
        ("协议", run.manifest.protocol_version.clone()),
        ("世代", run.manifest.lifecycle.clone()),
        ("传输", run.manifest.transport.clone()),
        ("能力指纹", run.manifest.capability_hash.clone()),
        ("套件指纹", run.manifest.suite_hash.clone()),
        ("种子", run.manifest.seed.to_string()),
    ] {
        html.push_str(&format!(
            "<tr><th>{}</th><td>{}</td></tr>",
            escape(key),
            escape(&value)
        ));
    }
    html.push_str("</tbody></table>");

    html.push_str("<h2>用例结果</h2><table><thead><tr><th>状态</th><th>用例</th><th>方法/目标</th><th>耗时</th><th>失败分类</th></tr></thead><tbody>");
    for case in &report.cases {
        let s = &case.summary;
        html.push_str(&format!(
            "<tr><td class=\"{}\">{}</td><td>{}</td><td>{}<br><span class=\"muted\">{}</span></td><td>{}ms</td><td>{}</td></tr>",
            status_class(&s.status),
            status_label(&s.status),
            escape(&s.case_name),
            escape(method_label(&s.method)),
            escape(&s.target),
            s.duration_ms,
            s.failure_kind
                .as_ref()
                .map(failure_label)
                .unwrap_or("")
        ));
    }
    html.push_str("</tbody></table>");

    html.push_str("<h2>用例详情</h2>");
    for case in &report.cases {
        let s = &case.summary;
        html.push_str("<div class=\"case\">");
        html.push_str(&format!(
            "<div><span class=\"{}\">{}</span> <strong>{}</strong> <span class=\"muted\">{} · {} · {}ms · {} 次尝试</span></div>",
            status_class(&s.status),
            status_label(&s.status),
            escape(&s.case_name),
            escape(method_label(&s.method)),
            escape(&s.target),
            s.duration_ms,
            s.attempts
        ));
        if let Some(message) = &s.message {
            html.push_str(&format!("<div class=\"muted\">{}</div>", escape(message)));
        }
        if let Some(reason) = &s.skip_reason {
            html.push_str(&format!(
                "<div class=\"muted\">跳过原因：{}</div>",
                escape(skip_reason_label(reason))
            ));
        }
        if !case.assertions.is_empty() {
            html.push_str("<table><thead><tr><th>断言</th><th>结果</th><th>期望</th><th>实际</th><th>说明</th></tr></thead><tbody>");
            for assertion in &case.assertions {
                html.push_str(&format!(
                    "<tr><td>{}</td><td class=\"{}\">{}</td><td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>",
                    escape(&format!("{:?}", assertion.kind)),
                    if assertion.passed { "ok" } else { "bad" },
                    if assertion.passed { "通过" } else { "失败" },
                    escape(&compact(assertion.expected.as_ref())),
                    escape(&compact(assertion.actual.as_ref())),
                    escape(assertion.message.as_deref().unwrap_or(""))
                ));
            }
            html.push_str("</tbody></table>");
        }
        let evidence = redact_evidence(&case.evidence, include_sensitive);
        push_evidence_sections(&mut html, case, &evidence, include_sensitive);
        html.push_str("</div>");
    }
    html.push_str("<p class=\"muted\">由 MCP Tester 生成。报告默认脱敏；完整结果请查看本地调用记录。</p>");
    html.push_str("</div></body></html>");
    html
}

/// 生成 JUnit XML。
pub fn to_junit(report: &ReportData) -> String {
    let summary = &report.run.summary;
    let skipped = summary.skipped + summary.canceled;
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<testsuites tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">\n",
        summary.total,
        summary.failed,
        summary.error,
        skipped,
        summary.duration_ms as f64 / 1000.0
    ));
    xml.push_str(&format!(
        "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\" timestamp=\"{}\">\n",
        xml_escape(&report.run.suite_name),
        summary.total,
        summary.failed,
        summary.error,
        skipped,
        summary.duration_ms as f64 / 1000.0,
        xml_escape(&report.run.started_at.to_rfc3339())
    ));
    for (key, value) in [
        ("run_id", report.run.id.clone()),
        ("server", report.run.server_name.clone()),
        ("protocol", report.run.manifest.protocol_version.clone()),
        ("capability_hash", report.run.manifest.capability_hash.clone()),
        ("seed", report.run.manifest.seed.to_string()),
    ] {
        xml.push_str(&format!(
            "<property name=\"{}\" value=\"{}\"/>\n",
            xml_escape(key),
            xml_escape(&value)
        ));
    }
    for case in &report.cases {
        let s = &case.summary;
        xml.push_str(&format!(
            "<testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">\n",
            xml_escape(&s.case_name),
            xml_escape(&s.target),
            s.duration_ms as f64 / 1000.0
        ));
        match s.status {
            TestRunCaseStatus::Failed => {
                xml.push_str(&format!(
                    "<failure message=\"{}\">{}</failure>\n",
                    xml_escape(s.message.as_deref().unwrap_or("断言失败")),
                    xml_escape(&failure_detail(case))
                ));
            }
            TestRunCaseStatus::Error => {
                xml.push_str(&format!(
                    "<error message=\"{}\">{}</error>\n",
                    xml_escape(s.message.as_deref().unwrap_or("执行错误")),
                    xml_escape(&failure_detail(case))
                ));
            }
            TestRunCaseStatus::Skipped | TestRunCaseStatus::Canceled => {
                xml.push_str(&format!(
                    "<skipped message=\"{}\"/>\n",
                    xml_escape(
                        s.skip_reason
                            .as_deref()
                            .map(skip_reason_label)
                            .unwrap_or("跳过")
                    )
                ));
            }
            TestRunCaseStatus::Warning => {
                xml.push_str(&format!(
                    "<system-out>{}</system-out>\n",
                    xml_escape(s.message.as_deref().unwrap_or("警告"))
                ));
            }
            TestRunCaseStatus::Passed => {}
        }
        xml.push_str("</testcase>\n");
    }
    xml.push_str("</testsuite></testsuites>\n");
    xml
}

/// 生成 Markdown 摘要。
pub fn to_markdown(report: &ReportData) -> String {
    let summary = &report.run.summary;
    let mut md = String::new();
    md.push_str(&format!("# MCP Tester 测试报告 · {}\n\n", report.run.suite_name));
    md.push_str(&format!(
        "- 服务器：{}（{} / {}）\n- 开始：{}\n- 耗时：{}ms\n- 通过率：{:.1}%（有效 {:.1}%）\n\n",
        report.run.server_name,
        report.run.manifest.protocol_version,
        report.run.manifest.transport,
        report.run.started_at.to_rfc3339(),
        summary.duration_ms,
        summary.pass_rate * 100.0,
        summary.effective_pass_rate * 100.0
    ));
    md.push_str("| 总数 | 通过 | 失败 | 警告 | 跳过 | 错误 | 取消 |\n");
    md.push_str("|---|---|---|---|---|---|---|\n");
    md.push_str(&format!(
        "| {} | {} | {} | {} | {} | {} | {} |\n\n",
        summary.total,
        summary.passed,
        summary.failed,
        summary.warning,
        summary.skipped,
        summary.error,
        summary.canceled
    ));
    let failures: Vec<&TestRunCaseDetail> = report
        .cases
        .iter()
        .filter(|case| {
            matches!(
                case.summary.status,
                TestRunCaseStatus::Failed | TestRunCaseStatus::Error
            )
        })
        .collect();
    if failures.is_empty() {
        md.push_str("无失败用例。\n");
    } else {
        md.push_str("## 失败用例\n\n");
        md.push_str("| 用例 | 状态 | 失败分类 | 说明 |\n|---|---|---|---|\n");
        for case in failures {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                md_escape(&case.summary.case_name),
                status_label(&case.summary.status),
                case.summary
                    .failure_kind
                    .as_ref()
                    .map(failure_label)
                    .unwrap_or(""),
                md_escape(case.summary.message.as_deref().unwrap_or(""))
            ));
        }
    }
    md
}

/// 原子写文件（同目录临时文件 + rename）。
pub fn write(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("report")
    ));
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

fn report_value(report: &ReportData, include_sensitive: bool) -> Value {
    let cases: Vec<Value> = report
        .cases
        .iter()
        .map(|case| {
            json!({
                "run_case_id": case.summary.id,
                "case_id": case.summary.case_id,
                "name": case.summary.case_name,
                "method": case.summary.method,
                "target": case.summary.target,
                "status": case.summary.status,
                "failure_kind": case.summary.failure_kind,
                "skip_reason": case.summary.skip_reason,
                "duration_ms": case.summary.duration_ms,
                "attempts": case.summary.attempts,
                "call_record_id": case.summary.call_record_id,
                "message": case.summary.message,
                "assertions": case.assertions,
                "evidence": redact_evidence(&case.evidence, include_sensitive),
            })
        })
        .collect();
    json!({
        "report_schema_version": report.report_schema_version,
        "generated_at": report.generated_at,
        "run": report.run,
        "manifest": report.run.manifest,
        "summary": report.run.summary,
        "coverage": report.coverage,
        "cases": cases,
    })
}

fn redact_evidence(evidence: &Evidence, include_sensitive: bool) -> Value {
    let mut value = serde_json::to_value(evidence).unwrap_or(Value::Null);
    if !include_sensitive {
        redact_value(&mut value);
    }
    value
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *item = Value::String("******".to_string());
                } else {
                    redact_value(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        Value::String(text) => {
            if looks_sensitive(text) {
                *text = "******".to_string();
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["authorization", "cookie", "token", "api-key", "apikey", "secret", "password", "credential"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn looks_sensitive(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.starts_with("bearer ")
        || (text.starts_with("sk-") && text.len() > 12)
        || (text.matches('.').count() == 2
            && text.len() > 60
            && text.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_'))
}

fn push_evidence_sections(
    html: &mut String,
    case: &TestRunCaseDetail,
    evidence: &Value,
    include_sensitive: bool,
) {
    let summary = &case.summary;
    let request = evidence.get("request").unwrap_or(&Value::Null);
    let response = evidence.get("response_preview").unwrap_or(&Value::Null);
    let response_error = evidence.get("response_error").unwrap_or(&Value::Null);
    let omitted = evidence
        .get("omitted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let truncated = evidence
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    html.push_str("<details open><summary class=\"muted\">请求</summary>");
    html.push_str(&format!(
        "<div class=\"muted\">{} · {}</div>",
        escape(method_label(&summary.method)),
        escape(&summary.target)
    ));
    let arguments = case_arguments(case).or_else(|| {
        if request.is_null() {
            None
        } else {
            Some(request)
        }
    });
    if let Some(arguments) = arguments {
        let mut value = arguments.clone();
        if !include_sensitive {
            redact_value(&mut value);
        }
        html.push_str(&format!("<pre>{}</pre>", escape(&pretty(&value))));
    } else {
        html.push_str("<div class=\"muted\">无参数</div>");
    }
    if matches!(summary.status, TestRunCaseStatus::Skipped) {
        html.push_str("<div class=\"muted\">用例未执行</div>");
    }
    html.push_str("</details>");

    let open = if matches!(
        summary.status,
        TestRunCaseStatus::Failed | TestRunCaseStatus::Error | TestRunCaseStatus::Warning
    ) {
        " open"
    } else {
        ""
    };
    html.push_str(&format!(
        "<details{open}><summary class=\"muted\">响应</summary>"
    ));
    if !response_error.is_null() {
        let failed = matches!(
            summary.status,
            TestRunCaseStatus::Failed | TestRunCaseStatus::Error
        );
        let class = if failed { "bad" } else { "warn" };
        let label = if failed { "错误" } else { "预期协议错误" };
        html.push_str(&format!(
            "<div class=\"{class}\">{label}</div><pre>{}</pre>",
            escape(&pretty(response_error))
        ));
    }
    if !response.is_null() {
        html.push_str(&format!("<pre>{}</pre>", escape(&pretty(response))));
    } else if response_error.is_null() {
        html.push_str(&format!(
            "<div class=\"muted\">{}</div>",
            response_placeholder(&summary.status)
        ));
    }
    if truncated {
        html.push_str("<div class=\"warn\">结果已截断入库，完整内容见调用记录。</div>");
    }
    html.push_str("</details>");

    let wire = wire_payloads(evidence);
    html.push_str(&format!(
        "<details><summary class=\"muted\">报文（{}）</summary>",
        wire.len()
    ));
    if omitted {
        html.push_str("<div class=\"muted\">证据超过 16KB，已省略。</div>");
    } else if wire.is_empty() {
        html.push_str("<div class=\"muted\">无请求/响应报文。</div>");
    } else {
        for frame in wire {
            html.push_str(&format!("<pre>{}</pre>", escape(&pretty(frame))));
        }
    }
    html.push_str("</details>");

    push_list_section(
        html,
        "通知",
        evidence_list(evidence, "notifications"),
        omitted,
        "无通知。",
    );
    push_list_section(
        html,
        "stderr",
        evidence_list(evidence, "stderr"),
        omitted,
        "无 stderr 输出。",
    );
}

fn push_list_section(
    html: &mut String,
    title: &str,
    items: &[Value],
    omitted: bool,
    empty: &str,
) {
    html.push_str(&format!(
        "<details><summary class=\"muted\">{}（{}）</summary>",
        escape(title),
        items.len()
    ));
    if omitted {
        html.push_str("<div class=\"muted\">证据超过 16KB，已省略。</div>");
    } else if items.is_empty() {
        html.push_str(&format!("<div class=\"muted\">{}</div>", escape(empty)));
    } else {
        html.push_str(&format!(
            "<pre>{}</pre>",
            escape(&pretty(&Value::Array(items.to_vec())))
        ));
    }
    html.push_str("</details>");
}

fn case_arguments(case: &TestRunCaseDetail) -> Option<&Value> {
    case.case_snapshot
        .get("arguments")
        .filter(|value| !value.is_null())
}

fn response_placeholder(status: &TestRunCaseStatus) -> &'static str {
    match status {
        TestRunCaseStatus::Skipped => "用例未执行",
        TestRunCaseStatus::Canceled => "运行被取消",
        _ => "无响应体（服务端未返回）",
    }
}

fn evidence_list<'a>(evidence: &'a Value, key: &str) -> &'a [Value] {
    evidence
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn wire_payloads(evidence: &Value) -> Vec<&Value> {
    evidence_list(evidence, "wire")
        .iter()
        .filter(|entry| {
            matches!(
                entry.get("kind").and_then(Value::as_str),
                Some("request" | "response")
            )
        })
        .collect()
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn skip_reason_label(reason: &str) -> &'static str {
    match reason {
        "disabled" => "用例已禁用",
        "write_not_allowed" => "未允许写入",
        "unknown_not_allowed" => "未允许未知风险",
        "destructive_not_allowed" => "未允许破坏性操作",
        "target_missing" => "目标不存在",
        "after_cancel" => "运行取消后跳过",
        "after_failure" => "失败后跳过",
        "skipped" => "已跳过",
        _ => "已跳过（未知原因）",
    }
}

fn failure_detail(case: &TestRunCaseDetail) -> String {
    let assertions: Vec<String> = case
        .assertions
        .iter()
        .filter(|assertion| !assertion.passed)
        .map(|assertion| {
            format!(
                "{:?}: {}（期望 {}，实际 {}）",
                assertion.kind,
                assertion.message.as_deref().unwrap_or("失败"),
                compact(assertion.expected.as_ref()),
                compact(assertion.actual.as_ref())
            )
        })
        .collect();
    if assertions.is_empty() {
        case.summary.message.clone().unwrap_or_default()
    } else {
        assertions.join("; ")
    }
}

fn method_label(method: &super::models::TestMethod) -> &'static str {
    method.as_str()
}

fn status_label(status: &TestRunCaseStatus) -> &'static str {
    match status {
        TestRunCaseStatus::Passed => "通过",
        TestRunCaseStatus::Failed => "失败",
        TestRunCaseStatus::Warning => "警告",
        TestRunCaseStatus::Skipped => "跳过",
        TestRunCaseStatus::Error => "错误",
        TestRunCaseStatus::Canceled => "取消",
    }
}

fn status_class(status: &TestRunCaseStatus) -> &'static str {
    match status {
        TestRunCaseStatus::Passed => "passed",
        TestRunCaseStatus::Failed => "failed",
        TestRunCaseStatus::Warning => "warning",
        TestRunCaseStatus::Skipped => "skipped",
        TestRunCaseStatus::Error => "error",
        TestRunCaseStatus::Canceled => "skipped",
    }
}

fn failure_label(kind: &FailureKind) -> &'static str {
    match kind {
        FailureKind::AssertionFailed => "断言失败",
        FailureKind::InvalidInputAccepted => "非法输入被接受",
        FailureKind::ProtocolError => "协议错误",
        FailureKind::ToolErrorUnexpected => "非预期工具错误",
        FailureKind::TransportError => "传输错误",
        FailureKind::Timeout => "超时",
        FailureKind::ServerUnhealthy => "服务不健康",
        FailureKind::CaseInvalid => "用例无效",
        FailureKind::Unknown => "未知",
    }
}

fn compact(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let text = serde_json::to_string(value).unwrap_or_default();
    if text.chars().count() > 200 {
        format!("{}…", text.chars().take(200).collect::<String>())
    } else {
        text
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn xml_escape(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|ch| {
            matches!(*ch, '\t' | '\n' | '\r') || ('\u{20}'..='\u{D7FF}').contains(ch)
        })
        .collect();
    escape(&cleaned)
}

fn md_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::error::CallError;
    use crate::testing::models::{
        AssertionKind, AssertionResult, AssertionSeverity, Coverage, RunManifest, RunOptions,
        RunSummary, TestMethod, TestRun, TestRunCaseDetail, TestRunCaseSummary, TestRunStatus,
    };
    use chrono::Utc;
    use serde_json::json;

    fn report() -> ReportData {
        let now = Utc::now();
        let summary = RunSummary {
            total: 1,
            failed: 1,
            effective_pass_rate: 0.0,
            ..Default::default()
        };
        let run = TestRun {
            id: "run-1".to_string(),
            suite_id: Some("suite-1".to_string()),
            suite_name: "套件 <x>".to_string(),
            server_id: "s1".to_string(),
            server_name: "mock".to_string(),
            status: TestRunStatus::Failed,
            stop_reason: None,
            options: RunOptions::default(),
            manifest: RunManifest {
                app_version: "0.2.2".to_string(),
                server_id: "s1".to_string(),
                server_name: "mock".to_string(),
                protocol_version: "2026-07-28".to_string(),
                lifecycle: "modern".to_string(),
                transport: "stdio".to_string(),
                capability_hash: "hash".to_string(),
                capability_stale: false,
                suite_hash: "suite".to_string(),
                seed: 1,
                tool_count: 1,
                resource_count: 0,
                template_count: 0,
                prompt_count: 0,
                started_at: now,
                finished_at: None,
                options: RunOptions::default(),
            },
            summary,
            baseline_run_id: None,
            started_at: now,
            finished_at: None,
        };
        let case = TestRunCaseDetail {
            summary: TestRunCaseSummary {
                id: "rc1".to_string(),
                case_id: Some("c1".to_string()),
                case_name: "echo <script>".to_string(),
                method: TestMethod::ToolsCall,
                target: "echo".to_string(),
                order_index: 0,
                status: TestRunCaseStatus::Failed,
                failure_kind: Some(FailureKind::AssertionFailed),
                skip_reason: None,
                duration_ms: 12,
                attempts: 1,
                call_record_id: Some("c1".to_string()),
                message: Some("断言失败".to_string()),
            },
            case_snapshot: json!({
                "method": "tools_call",
                "target": "echo",
                "arguments": {"text": "hi", "Authorization": "Bearer secret"},
            }),
            attempt_record_ids: vec!["c1".to_string()],
            assertions: vec![AssertionResult {
                assertion_id: "a1".to_string(),
                kind: AssertionKind::JsonPath,
                severity: AssertionSeverity::Error,
                passed: false,
                expected: Some(json!(1)),
                actual: Some(json!(0)),
                message: Some("值不匹配".to_string()),
            }],
            evidence: Evidence {
                request: json!({"Authorization": "Bearer secret"}),
                response_preview: Some(json!({"text": "ok"})),
                ..Default::default()
            },
        };
        build(run, Coverage::default(), vec![case])
    }

    #[test]
    fn html_json_junit_escape_and_redact() {
        let report = report();
        let html = to_html(&report, false);
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("******"));
        assert!(!html.contains("Bearer secret"));

        let json = to_json(&report, false);
        assert!(json.contains("report_schema_version"));
        assert!(!json.contains("Bearer secret"));

        let junit = to_junit(&report);
        assert!(junit.contains("<testsuites"));
        assert!(junit.contains("<failure"));
        assert!(junit.contains("套件 &lt;x&gt;"));
    }

    #[test]
    fn html_has_five_evidence_sections() {
        let report = report();
        let html = to_html(&report, true);
        assert!(html.contains("<summary class=\"muted\">请求</summary>"));
        assert!(html.contains("<summary class=\"muted\">响应</summary>"));
        assert!(html.contains("<summary class=\"muted\">报文（"));
        assert!(html.contains("<summary class=\"muted\">通知（"));
        assert!(html.contains("<summary class=\"muted\">stderr（"));
        assert!(html.contains("警告"));
        assert!(!html.contains("请求 / 响应 / 证据"));
        assert!(html.contains("tools_call"));
        assert!(html.contains("&quot;text&quot;: &quot;hi&quot;"));
    }

    #[test]
    fn html_response_error_color_depends_on_status() {
        let mut report = report();
        report.cases[0].evidence.response_preview = None;
        report.cases[0].evidence.response_error = Some(CallError::Timeout { timeout_secs: 5 });

        report.cases[0].summary.status = TestRunCaseStatus::Failed;
        let html = to_html(&report, true);
        assert!(html.contains("class=\"bad\">错误"));
        assert!(html.contains("timeout"));

        report.cases[0].summary.status = TestRunCaseStatus::Passed;
        let html = to_html(&report, true);
        assert!(html.contains("class=\"warn\">预期协议错误"));
        assert!(!html.contains("class=\"bad\">错误"));
    }

    #[test]
    fn html_wire_excludes_notifications() {
        let mut report = report();
        report.cases[0].evidence.wire = serde_json::from_value(json!([
            {
                "seq": 1,
                "direction": "client_to_server",
                "kind": "request",
                "method": "tools/call",
                "rpc_id": "1",
                "summary": "request tools/call",
                "payload": {"jsonrpc": "2.0", "id": 1, "method": "tools/call"},
                "truncated": false,
                "bytes": 42,
                "is_error": false,
                "duration_ms": null,
                "at": "2026-10-22T00:00:00Z"
            },
            {
                "seq": 2,
                "direction": "server_to_client",
                "kind": "notification",
                "method": "notifications/message",
                "rpc_id": null,
                "summary": "notification",
                "payload": {"jsonrpc": "2.0", "method": "notifications/message"},
                "truncated": false,
                "bytes": 21,
                "is_error": false,
                "duration_ms": null,
                "at": "2026-10-22T00:00:01Z"
            }
        ]))
        .unwrap();
        let html = to_html(&report, true);
        assert!(html.contains("报文（1）"));
        let start = html.find("报文（1）").unwrap();
        let end = html[start..].find("</details>").unwrap() + start;
        assert!(!html[start..end].contains("notifications/message"));
    }

    #[test]
    fn junit_skip_reason_is_chinese() {
        let mut report = report();
        report.cases[0].summary.status = TestRunCaseStatus::Skipped;
        report.cases[0].summary.skip_reason = Some("write_not_allowed".to_string());
        let junit = to_junit(&report);
        assert!(junit.contains("未允许写入"));
        assert!(!junit.contains("write_not_allowed"));
    }

    #[test]
    fn old_evidence_without_response_error_deserializes() {
        let evidence: Evidence = serde_json::from_value(json!({
            "request": {},
            "truncated": false,
            "wire": [],
            "notifications": [],
            "stderr": [],
            "omitted": false
        }))
        .unwrap();
        assert!(evidence.response_error.is_none());
    }

    #[test]
    fn markdown_escapes_pipes() {
        let mut report = report();
        report.cases[0].summary.case_name = "a|b".to_string();
        let md = to_markdown(&report);
        assert!(md.contains("a\\|b"));
    }
}
