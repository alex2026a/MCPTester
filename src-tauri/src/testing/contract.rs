//! 契约快照、稳定指纹与漂移对比。

use chrono::Utc;
use serde_json::{Map, Value, json};

use crate::mcp::models::CapabilitySnapshot;

use super::models::{ContractChange, ContractDiff, ContractSnapshot};

/// 计算能力快照的稳定指纹（128-bit FNV-1a，32 位十六进制）。
pub fn capability_hash(snapshot: &CapabilitySnapshot) -> String {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.remove("fetched_at");
        object.remove("stale");
        // 降级提示属于本次枚举的临时诊断信息，不属于能力契约。
        object.remove("warnings");
    }
    value_hash(&value)
}

/// 计算任意 JSON 的稳定指纹。
pub fn value_hash(value: &Value) -> String {
    fnv1a_128_hex(canonical_json(value).as_bytes())
}

/// 保存到套件/运行清单的快照值（保留 fetched_at/stale，便于反序列化与审计）。
pub fn snapshot_value(snapshot: &CapabilitySnapshot) -> Value {
    serde_json::to_value(snapshot).unwrap_or(Value::Null)
}

/// 构造契约快照。
pub fn build_snapshot(snapshot: &CapabilitySnapshot) -> ContractSnapshot {
    ContractSnapshot {
        server_id: snapshot.server_id.clone(),
        capability_hash: capability_hash(snapshot),
        fetched_at: snapshot.fetched_at,
        stale: snapshot.stale,
        tool_count: snapshot.tools.len() as u64,
        resource_count: snapshot.resources.len() as u64,
        template_count: snapshot.resource_templates.len() as u64,
        prompt_count: snapshot.prompts.len() as u64,
        snapshot: snapshot_value(snapshot),
    }
}

/// 对比套件保存的快照与当前能力快照。
pub fn diff_contract(old: &Value, current: &CapabilitySnapshot) -> ContractDiff {
    let old: CapabilitySnapshot = serde_json::from_value(old.clone()).unwrap_or(CapabilitySnapshot {
        server_id: current.server_id.clone(),
        tools: Vec::new(),
        resources: Vec::new(),
        resource_templates: Vec::new(),
        prompts: Vec::new(),
        fetched_at: Utc::now(),
        stale: true,
        warnings: Vec::new(),
    });

    let mut diff = ContractDiff {
        changed: false,
        added: Vec::new(),
        removed: Vec::new(),
        modified: Vec::new(),
    };

    compare_group(
        "tool",
        old.tools.iter().map(|item| (item.name.clone(), item)).collect(),
        current
            .tools
            .iter()
            .map(|item| (item.name.clone(), item))
            .collect(),
        &mut diff,
    );
    compare_group(
        "resource",
        old.resources.iter().map(|item| (item.uri.clone(), item)).collect(),
        current
            .resources
            .iter()
            .map(|item| (item.uri.clone(), item))
            .collect(),
        &mut diff,
    );
    compare_group(
        "template",
        old.resource_templates
            .iter()
            .map(|item| (item.uri_template.clone(), item))
            .collect(),
        current
            .resource_templates
            .iter()
            .map(|item| (item.uri_template.clone(), item))
            .collect(),
        &mut diff,
    );
    compare_group(
        "prompt",
        old.prompts.iter().map(|item| (item.name.clone(), item)).collect(),
        current
            .prompts
            .iter()
            .map(|item| (item.name.clone(), item))
            .collect(),
        &mut diff,
    );

    diff.changed = !(diff.added.is_empty() && diff.removed.is_empty() && diff.modified.is_empty());
    diff
}

fn compare_group<T: serde::Serialize>(
    kind: &str,
    old: std::collections::BTreeMap<String, &T>,
    current: std::collections::BTreeMap<String, &T>,
    diff: &mut ContractDiff,
) {
    for key in old.keys() {
        if !current.contains_key(key) {
            diff.removed.push(format!("{kind}/{key}"));
        }
    }
    for (key, current_item) in &current {
        match old.get(key) {
            None => diff.added.push(format!("{kind}/{key}")),
            Some(old_item) => {
                let old_value = serde_json::to_value(old_item).unwrap_or(Value::Null);
                let new_value = serde_json::to_value(current_item).unwrap_or(Value::Null);
                if canonical_json(&old_value) != canonical_json(&new_value) {
                    let fields = changed_fields(&old_value, &new_value);
                    diff.modified.push(ContractChange {
                        kind: kind.to_string(),
                        name: key.clone(),
                        fields,
                    });
                }
            }
        }
    }
}

fn changed_fields(old: &Value, new: &Value) -> Vec<String> {
    const FIELDS: &[&str] = &[
        "title",
        "description",
        "input_schema",
        "output_schema",
        "annotations",
        "mime_type",
        "size",
        "arguments",
        "uri_template",
    ];
    let mut fields = Vec::new();
    if let (Some(old), Some(new)) = (old.as_object(), new.as_object()) {
        for field in FIELDS {
            if canonical_json(old.get(*field).unwrap_or(&Value::Null))
                != canonical_json(new.get(*field).unwrap_or(&Value::Null))
            {
                fields.push((*field).to_string());
            }
        }
    }
    if fields.is_empty() {
        fields.push("definition".to_string());
    }
    fields
}

/// 递归排序对象键后紧凑序列化，保证跨平台/跨次运行稳定。
pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&sorted(value)).unwrap_or_default()
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn fnv1a_128_hex(bytes: &[u8]) -> String {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:032x}")
}

/// 用例套件指纹：对启用用例的有序定义做规范化后取指纹。
pub fn suite_hash(cases: &[super::models::TestCase]) -> String {
    let value = Value::Array(
        cases
            .iter()
            .filter(|case| case.enabled)
            .map(|case| {
                json!({
                    "method": case.method,
                    "target": case.target,
                    "arguments": case.arguments,
                    "expect": serde_json::to_value(&case.expect).unwrap_or(Value::Null),
                    "assertions": serde_json::to_value(&case.assertions).unwrap_or(Value::Null),
                    "tags": case.tags,
                    "priority": case.priority,
                    "risk": case.risk,
                })
            })
            .collect(),
    );
    value_hash(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{PromptInfo, ResourceInfo, ToolInfo, TransportKind};
    use serde_json::json;

    fn snapshot() -> CapabilitySnapshot {
        CapabilitySnapshot {
            server_id: "s1".to_string(),
            tools: vec![ToolInfo {
                name: "echo".to_string(),
                title: None,
                description: Some("回显".to_string()),
                input_schema: Some(json!({"type": "object"})),
                output_schema: None,
                annotations: None,
            }],
            resources: vec![ResourceInfo {
                uri: "mock://hello".to_string(),
                name: "hello".to_string(),
                title: None,
                description: None,
                mime_type: Some("text/plain".to_string()),
                size: None,
                annotations: None,
            }],
            resource_templates: Vec::new(),
            prompts: vec![PromptInfo {
                name: "greet".to_string(),
                title: None,
                description: None,
                arguments: Vec::new(),
            }],
            fetched_at: Utc::now(),
            stale: false,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn hash_is_stable_and_meta_independent() {
        let a = snapshot();
        let mut b = snapshot();
        b.fetched_at = Utc::now();
        b.stale = true;
        b.warnings.push("临时降级提示".to_string());
        assert_eq!(capability_hash(&a), capability_hash(&b));
        assert_eq!(capability_hash(&a).len(), 32);
    }

    #[test]
    fn old_snapshot_without_warnings_deserializes() {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value
            .as_object_mut()
            .expect("快照应为对象")
            .remove("warnings");
        let parsed: CapabilitySnapshot = serde_json::from_value(value).unwrap();
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let left = json!({"b": 1, "a": {"d": 2, "c": 3}});
        let right = json!({"a": {"c": 3, "d": 2}, "b": 1});
        assert_eq!(canonical_json(&left), canonical_json(&right));
    }

    #[test]
    fn diff_detects_added_removed_modified() {
        let old = snapshot();
        let old_value = snapshot_value(&old);
        let mut current = snapshot();
        current.tools[0].description = Some("新描述".to_string());
        current.tools.push(ToolInfo {
            name: "add".to_string(),
            title: None,
            description: None,
            input_schema: Some(json!({"type": "object"})),
            output_schema: None,
            annotations: None,
        });
        current.resources.clear();
        current.resource_templates.push(crate::mcp::models::ResourceTemplateInfo {
            uri_template: "mock://item/{id}".to_string(),
            name: "item".to_string(),
            title: None,
            description: None,
            mime_type: None,
        });
        let diff = diff_contract(&old_value, &current);
        assert!(diff.changed);
        assert!(diff.added.contains(&"tool/add".to_string()));
        assert!(diff.removed.contains(&"resource/mock://hello".to_string()));
        assert!(diff.modified.iter().any(|item| item.name == "echo"));
        assert!(diff.added.contains(&"template/mock://item/{id}".to_string()));
    }

    #[test]
    fn transport_serde_smoke() {
        // 防止测试模块未使用导入报错
        let _ = TransportKind::Stdio;
    }
}
