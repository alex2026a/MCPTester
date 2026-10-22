//! 确定性用例生成器（Schema + 历史样例；纯函数）。

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::mcp::models::{CapabilitySnapshot, PromptInfo, ResourceInfo, ToolInfo};

use super::contract::{build_snapshot, canonical_json, capability_hash};
use super::history::HistorySample;
use super::models::{
    AssertionKind, AssertionSeverity, GenerationOptions, GenerationPreview, GenerationProfile,
    GenerationScope, GenerationStats, TestAssertion, TestCaseDraft, TestExpect, TestOutcome,
    TestPriority, TestRisk, TestSource, TestMethod,
};
use super::schema;

/// 生成用例预览。
pub fn generate(
    snapshot: &CapabilitySnapshot,
    samples: &[HistorySample],
    options: &GenerationOptions,
) -> GenerationPreview {
    let mut warnings = Vec::new();
    let mut cases = Vec::new();
    let mut tools: Vec<&ToolInfo> = snapshot.tools.iter().collect();
    tools.sort_by(|left, right| left.name.cmp(&right.name));

    let include_tools = matches!(
        options.scope,
        GenerationScope::All | GenerationScope::Tools | GenerationScope::Current
    );
    if include_tools {
        for tool in tools {
            if options
                .target
                .as_deref()
                .is_some_and(|target| target != tool.name)
            {
                continue;
            }
            let mut tool_cases = generate_tool(tool, samples, options, &mut warnings);
            if tool_cases.len() > options.max_cases_per_tool as usize {
                warnings.push(format!(
                    "工具 {} 超过每工具上限 {}，已裁剪",
                    tool.name, options.max_cases_per_tool
                ));
                tool_cases.truncate(options.max_cases_per_tool as usize);
            }
            cases.extend(tool_cases);
        }
    }

    if options.include_resources
        && matches!(
            options.scope,
            GenerationScope::All | GenerationScope::Resources | GenerationScope::Current
        )
    {
        let mut resources: Vec<&ResourceInfo> = snapshot.resources.iter().collect();
        resources.sort_by(|left, right| left.uri.cmp(&right.uri));
        for resource in resources.into_iter().take(20) {
            if options
                .target
                .as_deref()
                .is_some_and(|target| target != resource.uri)
            {
                continue;
            }
            cases.push(resource_case(resource));
        }
        for template in snapshot.resource_templates.iter().take(10) {
            if options
                .target
                .as_deref()
                .is_some_and(|target| target != template.uri_template)
            {
                continue;
            }
            match instantiate_template(&template.uri_template) {
                Some(uri) => cases.push(TestCaseDraft {
                    name: format!("{} · 模板读取", template.name),
                    description: template.description.clone().unwrap_or_default(),
                    method: TestMethod::ResourcesRead,
                    target: uri,
                    arguments: json!({}),
                    expect: TestExpect::default(),
                    assertions: vec![shape_assertion(), content_assertion()],
                    tags: vec!["positive".to_string(), "resource_template".to_string()],
                    priority: TestPriority::P1,
                    risk: TestRisk::ReadOnly,
                    source: TestSource::Schema,
                    generation_note: Some(format!(
                        "由模板 {} 实例化",
                        template.uri_template
                    )),
                    enabled: true,
                    timeout_secs: None,
                    retry_count: 0,
                }),
                None => warnings.push(format!(
                    "模板 {} 无法解析变量，已跳过",
                    template.uri_template
                )),
            }
        }
        // 固定的非法 URI，保证确定性；没有任何资源时不生成。
        if (!snapshot.resources.is_empty() || !snapshot.resource_templates.is_empty())
            && snapshot.resources.len() < 20
        {
            cases.push(TestCaseDraft {
                name: "资源 · 非法 URI".to_string(),
                description: String::new(),
                method: TestMethod::ResourcesRead,
                target: "mcp://invalid/mcptester".to_string(),
                arguments: json!({}),
                expect: TestExpect {
                    outcome: TestOutcome::Rejected,
                    ..Default::default()
                },
                assertions: vec![shape_assertion()],
                tags: vec!["negative:missing_required".to_string(), "resource:invalid_uri".to_string()],
                priority: TestPriority::P2,
                risk: TestRisk::ReadOnly,
                source: TestSource::Schema,
                generation_note: Some("固定非法资源 URI，验证错误返回".to_string()),
                enabled: true,
                timeout_secs: None,
                retry_count: 0,
            });
        }
    }

    if options.include_prompts
        && matches!(
            options.scope,
            GenerationScope::All | GenerationScope::Prompts | GenerationScope::Current
        )
    {
        let mut prompts: Vec<&PromptInfo> = snapshot.prompts.iter().collect();
        prompts.sort_by(|left, right| left.name.cmp(&right.name));
        for prompt in prompts.into_iter().take(20) {
            if options
                .target
                .as_deref()
                .is_some_and(|target| target != prompt.name)
            {
                continue;
            }
            let mut prompt_cases = generate_prompt(prompt, options);
            cases.append(&mut prompt_cases);
        }
    }

    // 去重（历史优先）
    let mut seen = HashSet::new();
    cases.retain(|case| {
        let key = format!(
            "{}|{}|{}|{}",
            case.method.as_str(),
            case.target,
            canonical_json(&case.arguments),
            outcome_key(&case.expect.outcome)
        );
        seen.insert(key)
    });

    if cases.len() > options.max_total as usize {
        warnings.push(format!(
            "用例总数超过上限 {}，已从尾部裁剪",
            options.max_total
        ));
        cases.truncate(options.max_total as usize);
    }

    let stats = stats(&cases);
    GenerationPreview {
        server_id: snapshot.server_id.clone(),
        capability_hash: capability_hash(snapshot),
        capability_snapshot: build_snapshot(snapshot).snapshot,
        cases,
        warnings,
        stats,
    }
}

fn generate_tool(
    tool: &ToolInfo,
    samples: &[HistorySample],
    options: &GenerationOptions,
    warnings: &mut Vec<String>,
) -> Vec<TestCaseDraft> {
    let (risk, risk_warnings) = classify_risk(tool);
    warnings.extend(risk_warnings);
    let enabled = risk != TestRisk::Destructive || options.allow_destructive;
    let schema = tool
        .input_schema
        .as_ref()
        .filter(|value| schema::is_usable(value));
    let Some(schema) = schema else {
        return vec![TestCaseDraft {
            name: format!("{} · 手工占位", tool.name),
            description: tool.description.clone().unwrap_or_default(),
            method: TestMethod::ToolsCall,
            target: tool.name.clone(),
            arguments: json!({}),
            expect: TestExpect::default(),
            assertions: vec![shape_assertion()],
            tags: vec!["positive".to_string(), "needs-review".to_string()],
            priority: TestPriority::P1,
            risk,
            source: TestSource::Schema,
            generation_note: Some("无可用 inputSchema，参数为空，请人工复核".to_string()),
            enabled,
            timeout_secs: None,
            retry_count: 0,
        }];
    };

    let history = samples
        .iter()
        .find(|sample| sample.method == TestMethod::ToolsCall && sample.target == tool.name);
    let (mut base_args, source, note) = match history {
        Some(sample) if schema::validate(schema, &sample.arguments).valid => (
            sample.arguments.clone(),
            TestSource::History,
            format!("历史成功调用 {}", sample.record_id),
        ),
        Some(_) => {
            warnings.push(format!(
                "工具 {} 的历史参数不符合当前 Schema，回退 Schema 生成",
                tool.name
            ));
            let (value, warns) = schema::generate_positive(schema, options.seed);
            warnings.extend(warns);
            (value, TestSource::Schema, "由 inputSchema 生成".to_string())
        }
        None => {
            let (value, warns) = schema::generate_positive(schema, options.seed);
            warnings.extend(warns);
            (value, TestSource::Schema, "由 inputSchema 生成".to_string())
        }
    };
    if !base_args.is_object() {
        warnings.push(format!("工具 {} 生成参数不是对象，已改用空对象", tool.name));
        base_args = json!({});
    }

    let mut cases = Vec::new();
    cases.push(tool_case(
        tool,
        format!("{} · 正向基线", tool.name),
        base_args.clone(),
        TestExpect::default(),
        positive_assertions(tool),
        vec!["positive".to_string()],
        TestPriority::P0,
        risk.clone(),
        source,
        note,
        enabled,
    ));

    if options.profile != GenerationProfile::Smoke
        && let Some(Value::Object(properties)) = schema.get("properties")
        && !properties.is_empty()
        && optional_exists(schema, properties)
        && let Some((args, warns)) = schema::generate_with_optional(schema, options.seed)
    {
        warnings.extend(warns);
        if args != base_args {
            cases.push(tool_case(
                tool,
                format!("{} · 全部可选字段", tool.name),
                args,
                TestExpect::default(),
                positive_assertions(tool),
                vec!["positive".to_string(), "positive:optional".to_string()],
                TestPriority::P1,
                risk.clone(),
                TestSource::Schema,
                "包含全部可选字段".to_string(),
                enabled,
            ));
        }
    }

    if options.include_boundary && options.profile != GenerationProfile::Smoke {
        cases.extend(boundary_cases(
            tool,
            schema,
            &base_args,
            &risk,
            enabled,
            options,
            warnings,
        ));
    }
    if options.include_negative && options.profile != GenerationProfile::Smoke {
        cases.extend(negative_cases(
            tool,
            schema,
            &base_args,
            &risk,
            enabled,
            warnings,
        ));
    }
    cases
}

#[allow(clippy::too_many_arguments)]
fn tool_case(
    tool: &ToolInfo,
    name: String,
    arguments: Value,
    expect: TestExpect,
    assertions: Vec<TestAssertion>,
    tags: Vec<String>,
    priority: TestPriority,
    risk: TestRisk,
    source: TestSource,
    note: String,
    enabled: bool,
) -> TestCaseDraft {
    TestCaseDraft {
        name,
        description: tool.description.clone().unwrap_or_default(),
        method: TestMethod::ToolsCall,
        target: tool.name.clone(),
        arguments,
        expect,
        assertions,
        tags,
        priority,
        risk,
        source,
        generation_note: Some(note),
        enabled,
        timeout_secs: None,
        retry_count: 0,
    }
}

fn positive_assertions(tool: &ToolInfo) -> Vec<TestAssertion> {
    let mut assertions = vec![shape_assertion()];
    if tool.output_schema.is_some() {
        assertions.push(TestAssertion {
            id: "output_schema".to_string(),
            kind: AssertionKind::OutputSchema,
            severity: AssertionSeverity::Error,
            enabled: true,
            target: Some("structured_content".to_string()),
            path: None,
            op: None,
            expected: None,
            value_ms: None,
            message: Some("outputSchema 校验".to_string()),
            ignore_paths: Vec::new(),
            schema: None,
            schema_ref: Some("output_schema".to_string()),
        });
    }
    assertions
}

fn shape_assertion() -> TestAssertion {
    TestAssertion {
        id: "mcp_shape".to_string(),
        kind: AssertionKind::McpShape,
        severity: AssertionSeverity::Error,
        enabled: true,
        target: None,
        path: None,
        op: None,
        expected: None,
        value_ms: None,
        message: Some("MCP 结果基本形状".to_string()),
        ignore_paths: Vec::new(),
        schema: None,
        schema_ref: None,
    }
}

fn content_assertion() -> TestAssertion {
    TestAssertion {
        id: "content_non_empty".to_string(),
        kind: AssertionKind::Content,
        severity: AssertionSeverity::Warning,
        enabled: true,
        target: None,
        path: None,
        op: Some("regex".to_string()),
        expected: Some(Value::String(".+".to_string())),
        value_ms: None,
        message: Some("内容非空".to_string()),
        ignore_paths: Vec::new(),
        schema: None,
        schema_ref: None,
    }
}

fn optional_exists(schema: &Value, properties: &Map<String, Value>) -> bool {
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    properties
        .keys()
        .any(|key| !required.contains(key.as_str()))
}

fn boundary_cases(
    tool: &ToolInfo,
    schema: &Value,
    base: &Value,
    risk: &TestRisk,
    enabled: bool,
    options: &GenerationOptions,
    warnings: &mut Vec<String>,
) -> Vec<TestCaseDraft> {
    let mut cases = Vec::new();
    for (path, field_schema) in collect_fields(schema) {
        let Some(object) = field_schema.as_object() else {
            continue;
        };
        if let Some(min) = object.get("minimum").and_then(Value::as_f64) {
            push_boundary(
                &mut cases, tool, base, &path, json!(min), "min", true, risk, enabled,
            );
            push_boundary(
                &mut cases, tool, base, &path, json!(min - 1.0), "min-1", false, risk, enabled,
            );
        }
        if let Some(max) = object.get("maximum").and_then(Value::as_f64) {
            push_boundary(
                &mut cases, tool, base, &path, json!(max), "max", true, risk, enabled,
            );
            push_boundary(
                &mut cases, tool, base, &path, json!(max + 1.0), "max+1", false, risk, enabled,
            );
        }
        if let Some(min) = object.get("minLength").and_then(Value::as_u64) {
            let text = "a".repeat(min.min(4096) as usize);
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::String(text),
                "minLength",
                true,
                risk,
                enabled,
            );
            if min > 0 {
                let text = "a".repeat((min - 1).min(4096) as usize);
                push_boundary(
                    &mut cases,
                    tool,
                    base,
                    &path,
                    Value::String(text),
                    "minLength-1",
                    false,
                    risk,
                    enabled,
                );
            }
        }
        if let Some(max) = object.get("maxLength").and_then(Value::as_u64) {
            let text = "a".repeat(max.min(4096) as usize);
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::String(text),
                "maxLength",
                true,
                risk,
                enabled,
            );
            if max < 4096 {
                let text = "a".repeat((max + 1).min(4096) as usize);
                push_boundary(
                    &mut cases,
                    tool,
                    base,
                    &path,
                    Value::String(text),
                    "maxLength+1",
                    false,
                    risk,
                    enabled,
                );
            }
        }
        if let Some(Value::Array(values)) = object.get("enum") {
            for (index, value) in values.iter().take(5).enumerate() {
                push_boundary(
                    &mut cases,
                    tool,
                    base,
                    &path,
                    value.clone(),
                    &format!("enum{index}"),
                    true,
                    risk,
                    enabled,
                );
            }
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::String("__invalid_enum__".to_string()),
                "enum-invalid",
                false,
                risk,
                enabled,
            );
        }
        if let Some(min) = object.get("minItems").and_then(Value::as_u64)
            && min <= 20
        {
            let items = vec![Value::Null; min as usize];
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::Array(items),
                "minItems",
                true,
                risk,
                enabled,
            );
        }
        if let Some(max) = object.get("maxItems").and_then(Value::as_u64)
            && max <= 20
        {
            let items = vec![Value::Null; max as usize];
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::Array(items),
                "maxItems",
                true,
                risk,
                enabled,
            );
            let items = vec![Value::Null; (max + 1) as usize];
            push_boundary(
                &mut cases,
                tool,
                base,
                &path,
                Value::Array(items),
                "maxItems+1",
                false,
                risk,
                enabled,
            );
        }
    }
    if cases.len() > options.max_cases_per_tool as usize {
        warnings.push(format!(
            "工具 {} 边界用例超过上限，已裁剪",
            tool.name
        ));
        cases.truncate(options.max_cases_per_tool as usize);
    }
    cases
}

#[allow(clippy::too_many_arguments)]
fn push_boundary(
    cases: &mut Vec<TestCaseDraft>,
    tool: &ToolInfo,
    base: &Value,
    path: &[String],
    value: Value,
    label: &str,
    valid: bool,
    risk: &TestRisk,
    enabled: bool,
) {
    let mut arguments = base.clone();
    if !set_path(&mut arguments, path, value.clone()) {
        return;
    }
    let field = path.join(".");
    cases.push(tool_case(
        tool,
        format!("{} · 边界 · {field}={}", tool.name, compact(&value)),
        arguments,
        if valid {
            TestExpect::default()
        } else {
            TestExpect {
                outcome: TestOutcome::Rejected,
                ..Default::default()
            }
        },
        vec![shape_assertion()],
        vec![
            if valid {
                "boundary:min".to_string()
            } else {
                "boundary:invalid".to_string()
            },
            "boundary".to_string(),
        ],
        if valid {
            TestPriority::P1
        } else {
            TestPriority::P2
        },
        risk.clone(),
        TestSource::Schema,
        format!("{field} 的 {label} 边界"),
        enabled,
    ));
}

fn negative_cases(
    tool: &ToolInfo,
    schema: &Value,
    base: &Value,
    risk: &TestRisk,
    enabled: bool,
    warnings: &mut Vec<String>,
) -> Vec<TestCaseDraft> {
    let mut cases = Vec::new();
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(ToString::to_string).collect())
        .unwrap_or_default();
    for key in &required {
        let mut args = base.clone();
        if let Some(object) = args.as_object_mut() {
            object.remove(key);
        }
        cases.push(negative_case(
            tool,
            args,
            format!("缺少必填字段 {key}"),
            "negative:missing_required",
            risk,
            enabled,
        ));
    }
    for (path, field_schema) in collect_fields(schema) {
        let Some(object) = field_schema.as_object() else {
            continue;
        };
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !required.contains(&path[0]) && path.len() == 1 {
            continue;
        }
        let mut type_args = base.clone();
        if set_path(&mut type_args, &path, wrong_type(kind)) {
            cases.push(negative_case(
                tool,
                type_args,
                format!("字段 {} 类型错误", path.join(".")),
                "negative:type",
                risk,
                enabled,
            ));
        }
        if kind != "null"
            && object.get("nullable").and_then(Value::as_bool) != Some(true)
        {
            let mut null_args = base.clone();
            if set_path(&mut null_args, &path, Value::Null) {
                cases.push(negative_case(
                    tool,
                    null_args,
                    format!("字段 {} 为 null", path.join(".")),
                    "negative:null",
                    risk,
                    enabled,
                ));
            }
        }
    }
    if schema
        .get("additionalProperties")
        .and_then(Value::as_bool)
        == Some(false)
    {
        let mut args = base.clone();
        if let Some(object) = args.as_object_mut() {
            object.insert("__mcptester_extra__".to_string(), Value::Bool(true));
            cases.push(negative_case(
                tool,
                args,
                "额外字段".to_string(),
                "negative:additional_property",
                risk,
                enabled,
            ));
        }
    }
    cases.sort_by(|left, right| left.name.cmp(&right.name));
    if cases.len() > 12 {
        warnings.push(format!("工具 {} 负向用例超过上限，已裁剪", tool.name));
        cases.truncate(12);
    }
    cases
}

fn negative_case(
    tool: &ToolInfo,
    arguments: Value,
    note: String,
    tag: &str,
    risk: &TestRisk,
    enabled: bool,
) -> TestCaseDraft {
    tool_case(
        tool,
        format!("{} · 负向 · {note}", tool.name),
        arguments,
        TestExpect {
            outcome: TestOutcome::Rejected,
            ..Default::default()
        },
        vec![shape_assertion()],
        vec![tag.to_string(), "negative".to_string()],
        TestPriority::P1,
        risk.clone(),
        TestSource::Schema,
        note,
        enabled,
    )
}

fn collect_fields(schema: &Value) -> Vec<(Vec<String>, Value)> {
    let mut fields = Vec::new();
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return fields;
    };
    for (key, field_schema) in properties {
        fields.push((vec![key.clone()], field_schema.clone()));
        if let Some(nested) = field_schema.get("properties").and_then(Value::as_object) {
            for (nested_key, nested_schema) in nested {
                fields.push((
                    vec![key.clone(), nested_key.clone()],
                    nested_schema.clone(),
                ));
            }
        }
    }
    fields
}

fn set_path(value: &mut Value, path: &[String], replacement: Value) -> bool {
    if path.is_empty() {
        *value = replacement;
        return true;
    }
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if path.len() == 1 {
        object.insert(path[0].clone(), replacement);
        return true;
    }
    let Some(child) = object.get_mut(&path[0]) else {
        return false;
    };
    set_path(child, &path[1..], replacement)
}

fn wrong_type(kind: &str) -> Value {
    match kind {
        "string" => json!(123),
        "integer" | "number" => json!("not-a-number"),
        "boolean" => json!("not-a-boolean"),
        "array" => json!({}),
        "object" => json!([]),
        _ => Value::Null,
    }
}

fn generate_prompt(prompt: &PromptInfo, options: &GenerationOptions) -> Vec<TestCaseDraft> {
    let required: Map<String, Value> = prompt
        .arguments
        .iter()
        .filter(|arg| arg.required)
        .map(|arg| (arg.name.clone(), Value::String("test".to_string())))
        .collect();
    let mut all = required.clone();
    for arg in &prompt.arguments {
        all.entry(arg.name.clone())
            .or_insert_with(|| Value::String("test".to_string()));
    }
    let mut cases = vec![TestCaseDraft {
        name: format!("{} · 正向基线", prompt.name),
        description: prompt.description.clone().unwrap_or_default(),
        method: TestMethod::PromptsGet,
        target: prompt.name.clone(),
        arguments: Value::Object(required.clone()),
        expect: TestExpect::default(),
        assertions: vec![shape_assertion(), content_assertion()],
        tags: vec!["positive".to_string(), "prompt".to_string()],
        priority: TestPriority::P1,
        risk: TestRisk::ReadOnly,
        source: TestSource::Schema,
        generation_note: Some("必填参数最小集合".to_string()),
        enabled: true,
        timeout_secs: None,
        retry_count: 0,
    }];
    if all != required && options.profile != GenerationProfile::Smoke {
        cases.push(TestCaseDraft {
            name: format!("{} · 全部参数", prompt.name),
            description: prompt.description.clone().unwrap_or_default(),
            method: TestMethod::PromptsGet,
            target: prompt.name.clone(),
            arguments: Value::Object(all),
            expect: TestExpect::default(),
            assertions: vec![shape_assertion(), content_assertion()],
            tags: vec!["positive".to_string(), "prompt".to_string()],
            priority: TestPriority::P2,
            risk: TestRisk::ReadOnly,
            source: TestSource::Schema,
            generation_note: Some("全部参数".to_string()),
            enabled: true,
            timeout_secs: None,
            retry_count: 0,
        });
    }
    if options.include_negative && let Some(first) = prompt.arguments.iter().find(|arg| arg.required) {
        let mut missing = required.clone();
        missing.remove(&first.name);
        cases.push(TestCaseDraft {
            name: format!("{} · 负向 · 缺少 {}", prompt.name, first.name),
            description: prompt.description.clone().unwrap_or_default(),
            method: TestMethod::PromptsGet,
            target: prompt.name.clone(),
            arguments: Value::Object(missing),
            expect: TestExpect {
                outcome: TestOutcome::Rejected,
                ..Default::default()
            },
            assertions: vec![shape_assertion()],
            tags: vec!["negative:missing_required".to_string(), "prompt".to_string()],
            priority: TestPriority::P1,
            risk: TestRisk::ReadOnly,
            source: TestSource::Schema,
            generation_note: Some(format!("缺少必填参数 {}", first.name)),
            enabled: true,
            timeout_secs: None,
            retry_count: 0,
        });
    }
    cases
}

fn resource_case(resource: &ResourceInfo) -> TestCaseDraft {
    TestCaseDraft {
        name: format!("{} · 读取", resource.name),
        description: resource.description.clone().unwrap_or_default(),
        method: TestMethod::ResourcesRead,
        target: resource.uri.clone(),
        arguments: json!({}),
        expect: TestExpect::default(),
        assertions: vec![shape_assertion(), content_assertion()],
        tags: vec!["positive".to_string(), "resource".to_string()],
        priority: TestPriority::P1,
        risk: TestRisk::ReadOnly,
        source: TestSource::Schema,
        generation_note: Some(format!("读取已列资源 {}", resource.uri)),
        enabled: true,
        timeout_secs: None,
        retry_count: 0,
    }
}

fn instantiate_template(template: &str) -> Option<String> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }
        let mut expression = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            expression.push(next);
        }
        if !closed {
            return None;
        }
        let name = expression
            .trim_start_matches('+')
            .trim_start_matches('?')
            .trim_start_matches('#')
            .split(',')
            .next()
            .unwrap_or("id")
            .trim();
        if name.is_empty() {
            return None;
        }
        output.push_str(if name.eq_ignore_ascii_case("id") {
            "1"
        } else {
            "test"
        });
    }
    Some(output)
}

pub fn classify_risk(tool: &ToolInfo) -> (TestRisk, Vec<String>) {
    let mut warnings = Vec::new();
    let annotations = tool.annotations.as_ref().and_then(Value::as_object);
    let has_bool = annotations
        .map(|object| {
            ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"]
                .iter()
                .any(|key| object.get(*key).map(Value::is_boolean).unwrap_or(false))
        })
        .unwrap_or(false);
    let read_only = annotations
        .and_then(|object| object.get("readOnlyHint"))
        .and_then(Value::as_bool);
    let destructive = annotations
        .and_then(|object| object.get("destructiveHint"))
        .and_then(Value::as_bool);
    let hard = contains_word(&tool.name, HARD_DESTRUCTIVE);

    if !has_bool {
        return (if hard { TestRisk::Destructive } else { TestRisk::Unknown }, warnings);
    }
    if read_only == Some(true) {
        if hard {
            warnings.push(format!(
                "工具 {} 标注只读但名称含破坏性动词，风险降为 unknown",
                tool.name
            ));
            return (TestRisk::Unknown, warnings);
        }
        if destructive == Some(true) {
            warnings.push(format!(
                "工具 {} 同时标注 readOnlyHint=true 与 destructiveHint=true",
                tool.name
            ));
        }
        return (TestRisk::ReadOnly, warnings);
    }
    if destructive == Some(true) || (destructive.is_none() && read_only != Some(true)) {
        return (TestRisk::Destructive, warnings);
    }
    if hard {
        return (TestRisk::Destructive, warnings);
    }
    if destructive == Some(false) {
        return (TestRisk::Write, warnings);
    }
    (TestRisk::Unknown, warnings)
}

const HARD_DESTRUCTIVE: &[&str] = &[
    "delete", "remove", "drop", "truncate", "wipe", "purge", "erase", "destroy", "unlink",
    "revoke", "terminate", "shutdown", "kill", "pay", "charge", "refund", "transfer", "withdraw",
];
fn contains_word(name: &str, words: &[&str]) -> bool {
    let separated = camel_to_words(name);
    separated
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| words.contains(&word.to_ascii_lowercase().as_str()))
}

fn camel_to_words(name: &str) -> String {
    let mut output = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() && previous_lower {
            output.push('_');
        }
        output.push(ch);
        previous_lower = ch.is_lowercase() || ch.is_ascii_digit();
    }
    output
}

fn outcome_key(outcome: &TestOutcome) -> &'static str {
    match outcome {
        TestOutcome::Success => "success",
        TestOutcome::ToolError => "tool_error",
        TestOutcome::RpcError => "rpc_error",
        TestOutcome::Rejected => "rejected",
        TestOutcome::Any => "any",
    }
}

fn stats(cases: &[TestCaseDraft]) -> GenerationStats {
    let mut stats = GenerationStats {
        total: cases.len() as u32,
        ..Default::default()
    };
    for case in cases {
        if case.tags.iter().any(|tag| tag == "positive") {
            stats.positive += 1;
        }
        if case.tags.iter().any(|tag| tag.starts_with("boundary")) {
            stats.boundary += 1;
        }
        if case.tags.iter().any(|tag| tag.starts_with("negative")) {
            stats.negative += 1;
        }
        if case.tags.iter().any(|tag| tag == "resource" || tag == "resource_template") {
            stats.resources += 1;
        }
        if case.tags.iter().any(|tag| tag == "prompt") {
            stats.prompts += 1;
        }
        match case.risk {
            TestRisk::ReadOnly => stats.read_only += 1,
            TestRisk::Write => stats.write += 1,
            TestRisk::Unknown => stats.unknown += 1,
            TestRisk::Destructive => stats.destructive += 1,
        }
    }
    stats
}

fn compact(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    if text.chars().count() > 48 {
        format!("{}…", text.chars().take(48).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{CapabilitySnapshot, ToolInfo};
    use chrono::Utc;
    use serde_json::json;

    fn snapshot(tool: ToolInfo) -> CapabilitySnapshot {
        CapabilitySnapshot {
            server_id: "s1".to_string(),
            tools: vec![tool],
            resources: Vec::new(),
            resource_templates: Vec::new(),
            prompts: Vec::new(),
            fetched_at: Utc::now(),
            stale: false,
            warnings: Vec::new(),
        }
    }

    fn tool(schema: Value) -> ToolInfo {
        ToolInfo {
            name: "bounded".to_string(),
            title: None,
            description: Some("边界测试".to_string()),
            input_schema: Some(schema),
            output_schema: None,
            annotations: Some(json!({"readOnlyHint": true})),
        }
    }

    #[test]
    fn generates_positive_boundary_and_negative_deterministically() {
        let tool = tool(json!({
            "type": "object",
            "required": ["name", "count"],
            "properties": {
                "name": {"type": "string", "minLength": 2},
                "count": {"type": "integer", "minimum": 1, "maximum": 5},
                "tag": {"enum": ["a", "b"]}
            },
            "additionalProperties": false
        }));
        let options = GenerationOptions::default();
        let first = generate(&snapshot(tool.clone()), &[], &options);
        let second = generate(&snapshot(tool), &[], &options);
        assert_eq!(first.cases.len(), second.cases.len());
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
                .collect::<Vec<_>>()
        );
        assert!(first.stats.positive >= 1);
        assert!(first.stats.boundary >= 1);
        assert!(first.stats.negative >= 1);
        assert!(first.cases.iter().any(|case| case.expect.outcome == TestOutcome::Rejected));
    }

    #[test]
    fn empty_schema_is_placeholder() {
        let preview = generate(
            &snapshot(tool(json!({}))),
            &[],
            &GenerationOptions::default(),
        );
        assert_eq!(preview.cases.len(), 1);
        assert!(preview.cases[0].generation_note.as_deref().unwrap().contains("无可用"));
    }

    #[test]
    fn destructive_risk_is_disabled_by_default() {
        let mut tool = tool(json!({"type": "object"}));
        tool.name = "delete_file".to_string();
        tool.annotations = Some(json!({"destructiveHint": true}));
        let preview = generate(&snapshot(tool), &[], &GenerationOptions::default());
        assert_eq!(preview.stats.destructive, 1);
        assert!(!preview.cases[0].enabled);
    }

    #[test]
    fn template_instantiation() {
        assert_eq!(
            instantiate_template("mock://item/{id}").as_deref(),
            Some("mock://item/1")
        );
        assert_eq!(
            instantiate_template("file:///{path}").as_deref(),
            Some("file:///test")
        );
        assert!(instantiate_template("file:///{").is_none());
    }
}
