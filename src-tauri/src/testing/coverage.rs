//! 覆盖率计算（用例覆盖，不是代码覆盖率）。

use std::collections::HashSet;

use crate::mcp::models::CapabilitySnapshot;

use super::models::{Coverage, CoverageCategory, TestCase};

const NEGATIVE_CATEGORIES: &[&str] = &[
    "missing_required",
    "type",
    "null",
    "range",
    "enum",
    "format",
    "additional_property",
    "nested",
];

pub fn compute(cases: &[TestCase], snapshot: &CapabilitySnapshot) -> Coverage {
    let selected: Vec<&TestCase> = cases.iter().filter(|case| case.enabled).collect();
    let mut tool_targets = HashSet::new();
    let mut resource_targets = HashSet::new();
    let mut prompt_targets = HashSet::new();
    for case in &selected {
        match case.method {
            super::models::TestMethod::ToolsCall => {
                tool_targets.insert(case.target.clone());
            }
            super::models::TestMethod::ResourcesRead => {
                resource_targets.insert(case.target.clone());
            }
            super::models::TestMethod::PromptsGet => {
                prompt_targets.insert(case.target.clone());
            }
        }
    }

    let mut required_total = 0u64;
    let mut required_covered = 0u64;
    let mut constraints_total = 0u64;
    for tool in &snapshot.tools {
        let Some(schema) = tool.input_schema.as_ref() else {
            continue;
        };
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            for key in required.iter().filter_map(serde_json::Value::as_str) {
                required_total += 1;
                let covered = selected.iter().any(|case| {
                    case.method == super::models::TestMethod::ToolsCall
                        && case.target == tool.name
                        && case
                            .arguments
                            .get(key)
                            .is_some()
                });
                if covered {
                    required_covered += 1;
                }
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for property in properties.values() {
                for keyword in [
                    "minimum",
                    "maximum",
                    "minLength",
                    "maxLength",
                    "minItems",
                    "maxItems",
                    "enum",
                    "pattern",
                ] {
                    if property.get(keyword).is_some() {
                        constraints_total += 1;
                    }
                }
            }
        }
    }
    let constraints_covered = selected
        .iter()
        .filter(|case| case.tags.iter().any(|tag| tag.starts_with("boundary")))
        .count() as u64;

    let negative_categories = NEGATIVE_CATEGORIES
        .iter()
        .map(|name| {
            let tag = format!("negative:{name}");
            CoverageCategory {
                name: (*name).to_string(),
                covered: selected.iter().any(|case| case.tags.contains(&tag)),
                count: selected
                    .iter()
                    .filter(|case| case.tags.contains(&tag))
                    .count() as u64,
            }
        })
        .collect();

    Coverage {
        tools_total: snapshot.tools.len() as u64,
        tools_covered: tool_targets.len() as u64,
        resources_total: snapshot.resources.len() as u64,
        resources_covered: resource_targets.len() as u64,
        prompts_total: snapshot.prompts.len() as u64,
        prompts_covered: prompt_targets.len() as u64,
        required_params_total: required_total,
        required_params_covered: required_covered,
        constraints_total,
        constraints_covered,
        negative_categories,
        assertion_cases: selected
            .iter()
            .filter(|case| case.assertions.iter().any(|assertion| assertion.enabled))
            .count() as u64,
        selected_cases: selected.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{CapabilitySnapshot, ToolInfo};
    use crate::testing::models::{
        AssertionKind, AssertionSeverity, TestAssertion, TestCaseDraft, TestMethod, TestPriority,
        TestRisk, TestSource,
    };
    use chrono::Utc;
    use serde_json::json;

    fn snapshot() -> CapabilitySnapshot {
        CapabilitySnapshot {
            server_id: "s1".to_string(),
            tools: vec![ToolInfo {
                name: "echo".to_string(),
                title: None,
                description: None,
                input_schema: Some(json!({
                    "type": "object",
                    "required": ["text"],
                    "properties": {"text": {"type": "string", "minLength": 1}}
                })),
                output_schema: None,
                annotations: None,
            }],
            resources: Vec::new(),
            resource_templates: Vec::new(),
            prompts: Vec::new(),
            fetched_at: Utc::now(),
            stale: false,
            warnings: Vec::new(),
        }
    }

    fn case(target: &str, args: serde_json::Value) -> TestCase {
        let draft = TestCaseDraft {
            name: "case".to_string(),
            method: TestMethod::ToolsCall,
            target: target.to_string(),
            arguments: args,
            assertions: vec![TestAssertion {
                id: "a".to_string(),
                kind: AssertionKind::McpShape,
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
            }],
            tags: vec!["positive".to_string(), "boundary:min".to_string()],
            priority: TestPriority::P0,
            risk: TestRisk::ReadOnly,
            source: TestSource::Schema,
            ..Default::default()
        };
        TestCase {
            id: "c1".to_string(),
            suite_id: "suite".to_string(),
            name: draft.name,
            description: draft.description,
            method: draft.method,
            target: draft.target,
            arguments: draft.arguments,
            expect: draft.expect,
            assertions: draft.assertions,
            tags: draft.tags,
            priority: draft.priority,
            risk: draft.risk,
            source: draft.source,
            generation_note: draft.generation_note,
            enabled: true,
            order_index: 0,
            timeout_secs: None,
            retry_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn computes_interface_and_required_coverage() {
        let coverage = compute(&[case("echo", json!({"text": "hi"}))], &snapshot());
        assert_eq!(coverage.tools_total, 1);
        assert_eq!(coverage.tools_covered, 1);
        assert_eq!(coverage.required_params_total, 1);
        assert_eq!(coverage.required_params_covered, 1);
        assert_eq!(coverage.assertion_cases, 1);
        assert_eq!(coverage.selected_cases, 1);
    }
}
