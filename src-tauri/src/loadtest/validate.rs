//! 压测配置校验与风险快照（P0 只做纯函数；DB/manager 校验在 P1）。

use std::collections::HashSet;

use crate::mcp::models::TransportKind;
use crate::testing::models::{TestCase, TestRisk};

use super::models::{
    LoadCaseSnapshot, LoadPreviewOperation, LoadProfile, LoadRiskSummary, LoadSessionMode,
    MAX_OPERATIONS, MAX_STAGE_SECS, MAX_STAGES, MAX_THINK_MS, MAX_TIMEOUT_SECS, MAX_TOTAL_SECS,
    MAX_WEIGHT, max_vus_for,
};

/// 校验 profile；成功返回最大目标 VU。
pub fn validate_profile(
    profile: &LoadProfile,
    transport: &TransportKind,
    oauth: bool,
) -> Result<u32, String> {
    if profile.name.trim().is_empty() {
        return Err("压测名称不能为空".to_string());
    }
    if profile.stages.is_empty() {
        return Err("至少需要一个阶段".to_string());
    }
    if profile.stages.len() > MAX_STAGES {
        return Err(format!("阶段数不能超过 {MAX_STAGES}"));
    }
    let mut total_secs = 0u64;
    for (index, stage) in profile.stages.iter().enumerate() {
        if stage.duration_secs == 0 || stage.duration_secs > MAX_STAGE_SECS {
            return Err(format!(
                "第 {} 阶段时长必须在 1..={MAX_STAGE_SECS} 秒",
                index + 1
            ));
        }
        if stage.target_vus == 0 {
            return Err(format!("第 {} 阶段目标 VU 必须大于 0", index + 1));
        }
        total_secs = total_secs.saturating_add(stage.duration_secs);
    }
    if total_secs > MAX_TOTAL_SECS {
        return Err(format!("总时长不能超过 {MAX_TOTAL_SECS} 秒"));
    }
    let max_vus = profile.max_vus();
    let cap = max_vus_for(transport, profile.session_mode == LoadSessionMode::Shared, oauth);
    if max_vus > cap {
        let mode = if profile.session_mode == LoadSessionMode::Shared {
            "shared"
        } else {
            "per_vu"
        };
        return Err(format!(
            "{mode} 模式下该传输最大 {cap} VU（当前 {max_vus}）"
        ));
    }
    if profile.think_time_min_ms > profile.think_time_max_ms {
        return Err("think time 最小值不能大于最大值".to_string());
    }
    if profile.think_time_max_ms > MAX_THINK_MS {
        return Err(format!("think time 不能超过 {MAX_THINK_MS}ms"));
    }
    if profile.timeout_secs == 0 || profile.timeout_secs > MAX_TIMEOUT_SECS {
        return Err(format!("超时必须在 1..={MAX_TIMEOUT_SECS} 秒"));
    }
    if profile.operations.is_empty() {
        return Err("至少需要一个操作".to_string());
    }
    if profile.operations.len() > MAX_OPERATIONS {
        return Err(format!("操作数不能超过 {MAX_OPERATIONS}"));
    }
    let mut seen = HashSet::new();
    for operation in &profile.operations {
        if operation.case_id.trim().is_empty() {
            return Err("操作缺少 case_id".to_string());
        }
        if !seen.insert(operation.case_id.clone()) {
            return Err(format!("操作重复：{}", operation.case_id));
        }
        if operation.weight == 0 || operation.weight > MAX_WEIGHT {
            return Err(format!(
                "操作权重必须在 1..={MAX_WEIGHT}：{}",
                operation.case_id
            ));
        }
    }
    if let Some(rate) = profile.abort_error_rate
        && !(0.0..=1.0).contains(&rate)
    {
        return Err("错误率中止阈值必须在 0..=1".to_string());
    }
    if profile.abort_p99_ms == Some(0) {
        return Err("p99 中止阈值必须大于 0".to_string());
    }
    Ok(max_vus)
}

/// 把用例与权重合并成运行快照；缺失/重复由调用方先校验。
pub fn snapshot_operation(case: &TestCase, weight: u32) -> LoadCaseSnapshot {
    LoadCaseSnapshot {
        case_id: case.id.clone(),
        name: case.name.clone(),
        method: case.method.clone(),
        target: case.target.clone(),
        arguments: case.arguments.clone(),
        risk: case.risk.clone(),
        weight,
        timeout_secs: case.timeout_secs,
    }
}

/// 汇总有效风险（供 preview/报告）。
pub fn summarize_risks(operations: &[LoadCaseSnapshot]) -> LoadRiskSummary {
    let mut summary = LoadRiskSummary::default();
    for operation in operations {
        match operation.risk {
            TestRisk::ReadOnly => summary.read_only += 1,
            TestRisk::Write => summary.write += 1,
            TestRisk::Unknown => summary.unknown += 1,
            TestRisk::Destructive => summary.destructive += 1,
        }
    }
    summary
}

/// 按授权判断一个有效风险是否允许；返回拒绝原因。
pub fn reject_reason(
    risk: &TestRisk,
    allow_write: bool,
    allow_unknown: bool,
) -> Option<&'static str> {
    match risk {
        TestRisk::ReadOnly => None,
        TestRisk::Write if allow_write => None,
        TestRisk::Write => Some("write_not_allowed"),
        TestRisk::Unknown if allow_unknown => None,
        TestRisk::Unknown => Some("unknown_not_allowed"),
        TestRisk::Destructive => Some("destructive_not_allowed"),
    }
}

pub fn preview_operation(
    operation: &LoadCaseSnapshot,
    risk: TestRisk,
    allow_write: bool,
    allow_unknown: bool,
) -> LoadPreviewOperation {
    let reason = reject_reason(&risk, allow_write, allow_unknown).map(str::to_string);
    LoadPreviewOperation {
        case_id: operation.case_id.clone(),
        name: operation.name.clone(),
        method: operation.method.clone(),
        target: operation.target.clone(),
        weight: operation.weight,
        effective_risk: risk,
        allowed: reason.is_none(),
        reason,
    }
}

/// 按权重选择操作；纯函数便于固定 seed 测试。
pub fn pick_operation(operations: &[LoadCaseSnapshot], cursor: u64) -> usize {
    let total: u64 = operations.iter().map(|item| item.weight as u64).sum();
    if total == 0 {
        return 0;
    }
    let mut offset = cursor % total;
    for (index, operation) in operations.iter().enumerate() {
        let weight = operation.weight as u64;
        if offset < weight {
            return index;
        }
        offset -= weight;
    }
    operations.len().saturating_sub(1)
}

pub fn clamp_error_message(message: &str) -> String {
    let sanitized = crate::logging::scrub_text(&crate::mcp::oauth::sanitize_detail(message));
    let sanitized = redact_bearer(&sanitized);
    sanitized.chars().take(300).collect()
}

fn redact_bearer(text: &str) -> String {
    static BEARER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = BEARER.get_or_init(|| {
        regex::Regex::new(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]+")
            .expect("Bearer 脱敏正则编译失败")
    });
    regex.replace_all(text, "Bearer [REDACTED]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::TransportKind;
    use crate::testing::models::{TestOutcome, TestRisk};
    use serde_json::json;

    fn profile() -> LoadProfile {
        LoadProfile {
            name: "demo".to_string(),
            stages: vec![super::super::models::LoadStage {
                duration_secs: 10,
                target_vus: 2,
            }],
            operations: vec![super::super::models::LoadOperation {
                case_id: "c1".to_string(),
                weight: 1,
            }],
            ..Default::default()
        }
    }

    fn case(id: &str, risk: TestRisk) -> TestCase {
        TestCase {
            id: id.to_string(),
            suite_id: "s1".to_string(),
            name: id.to_string(),
            description: String::new(),
            method: crate::testing::models::TestMethod::ToolsCall,
            target: "echo".to_string(),
            arguments: json!({ "text": "hi" }),
            expect: crate::testing::models::TestExpect {
                outcome: TestOutcome::Success,
                ..Default::default()
            },
            assertions: Vec::new(),
            tags: Vec::new(),
            priority: crate::testing::models::TestPriority::P1,
            risk,
            source: crate::testing::models::TestSource::Manual,
            generation_note: None,
            enabled: true,
            order_index: 0,
            timeout_secs: None,
            retry_count: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn validates_limits_and_transport_caps() {
        let mut p = profile();
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_ok());
        p.stages[0].target_vus = 21;
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_err());
        p.stages[0].target_vus = 6;
        assert!(validate_profile(&p, &TransportKind::StreamableHttp, true).is_err());
        p.stages[0].target_vus = 100;
        p.session_mode = LoadSessionMode::Shared;
        assert!(validate_profile(&p, &TransportKind::StreamableHttp, true).is_ok());
    }

    #[test]
    fn rejects_bad_profile_fields() {
        let mut p = profile();
        p.stages[0].duration_secs = 0;
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_err());
        let mut p = profile();
        p.think_time_min_ms = 10;
        p.think_time_max_ms = 5;
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_err());
        let mut p = profile();
        p.operations[0].weight = 0;
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_err());
        let mut p = profile();
        p.abort_error_rate = Some(1.5);
        assert!(validate_profile(&p, &TransportKind::Stdio, false).is_err());
    }

    #[test]
    fn risk_gate_and_snapshot() {
        let c = case("c1", TestRisk::Write);
        let snap = snapshot_operation(&c, 2);
        assert_eq!(snap.weight, 2);
        assert_eq!(snap.arguments, json!({ "text": "hi" }));
        assert_eq!(reject_reason(&TestRisk::Write, false, false), Some("write_not_allowed"));
        assert_eq!(reject_reason(&TestRisk::Write, true, false), None);
        assert_eq!(
            reject_reason(&TestRisk::Destructive, true, true),
            Some("destructive_not_allowed")
        );
        let risk = summarize_risks(&[snap]);
        assert_eq!(risk.write, 1);
    }

    #[test]
    fn weighted_pick_is_stable() {
        let a = snapshot_operation(&case("a", TestRisk::ReadOnly), 1);
        let b = snapshot_operation(&case("b", TestRisk::ReadOnly), 3);
        let ops = vec![a, b];
        assert_eq!(pick_operation(&ops, 0), 0);
        assert_eq!(pick_operation(&ops, 1), 1);
        assert_eq!(pick_operation(&ops, 3), 1);
        assert_eq!(pick_operation(&ops, 4), 0);
    }

    #[test]
    fn message_is_sanitized_and_truncated() {
        let text = "Bearer abc ".to_string() + &"x".repeat(500);
        let out = clamp_error_message(&text);
        assert!(out.len() <= 300);
        assert!(!out.contains("Bearer abc"));
    }
}
