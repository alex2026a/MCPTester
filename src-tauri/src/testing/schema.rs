//! JSON Schema 常用子集：校验与确定性取值生成。
//!
//! 不追求完整 JSON Schema 规范；覆盖 MCP 工具常见的 2020-12 / draft-07 关键字。
//! 不支持的关键字只记 warning，不产生误判失败。

use std::collections::HashSet;

use regex::Regex;
use serde_json::{Map, Value, json};

/// 校验结果。
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    fn ok() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// 判断 Schema 是否足以生成参数。
pub fn is_usable(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object.is_empty() {
        return false;
    }
    object.contains_key("type")
        || object.contains_key("properties")
        || object.contains_key("required")
        || object.contains_key("oneOf")
        || object.contains_key("anyOf")
        || object.contains_key("allOf")
        || object.contains_key("$ref")
        || object.contains_key("enum")
        || object.contains_key("const")
}

/// 校验一个值（`root` 用于解析本地 `$ref`）。
pub fn validate(schema: &Value, value: &Value) -> ValidationReport {
    let mut report = ValidationReport {
        valid: true,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let mut seen = HashSet::new();
    validate_at(schema, value, schema, "$", 0, &mut seen, &mut report);
    report.valid = report.errors.is_empty();
    report
}

fn validate_at(
    schema: &Value,
    value: &Value,
    root: &Value,
    path: &str,
    depth: usize,
    seen: &mut HashSet<String>,
    report: &mut ValidationReport,
) {
    if depth > 32 {
        report.warnings.push(format!("{path}: Schema 嵌套过深，跳过校验"));
        return;
    }
    match schema {
        Value::Bool(true) | Value::Null => {}
        Value::Bool(false) => {
            report.errors.push(format!("{path}: Schema 为 false"));
        }
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                if reference.starts_with('#') {
                    let key = format!("{reference}@{path}");
                    if seen.insert(key) {
                        if let Some(target) = resolve_ref(root, reference) {
                            validate_at(target, value, root, path, depth + 1, seen, report);
                        } else {
                            report
                                .warnings
                                .push(format!("{path}: 无法解析 $ref {reference}"));
                        }
                    }
                } else {
                    report
                        .warnings
                        .push(format!("{path}: 不支持远程 $ref {reference}"));
                }
            }

            if let Some(expected) = object.get("type")
                && !type_matches(expected, value)
            {
                report
                    .errors
                    .push(format!("{path}: 类型不匹配，期望 {expected}"));
                return;
            }

            if let Some(constant) = object.get("const")
                && constant != value
            {
                report
                    .errors
                    .push(format!("{path}: 值应等于 const {constant}"));
            }

            if let Some(Value::Array(items)) = object.get("enum")
                && !items.iter().any(|item| item == value)
            {
                report
                    .errors
                    .push(format!("{path}: 值不在 enum 中"));
            }

            if let Some(Value::Array(items)) = object.get("allOf") {
                for item in items {
                    validate_at(item, value, root, path, depth + 1, seen, report);
                }
            }
            if let Some(Value::Array(items)) = object.get("anyOf") {
                let mut ok = false;
                for item in items {
                    let mut trial = ValidationReport::ok();
                    let mut trial_seen = HashSet::new();
                    validate_at(item, value, root, path, depth + 1, &mut trial_seen, &mut trial);
                    if trial.valid {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    report
                        .errors
                        .push(format!("{path}: 不满足 anyOf 任一分支"));
                }
            }
            if let Some(Value::Array(items)) = object.get("oneOf") {
                let mut matches = 0;
                for item in items {
                    let mut trial = ValidationReport::ok();
                    let mut trial_seen = HashSet::new();
                    validate_at(item, value, root, path, depth + 1, &mut trial_seen, &mut trial);
                    if trial.valid {
                        matches += 1;
                    }
                }
                if matches != 1 {
                    report
                        .errors
                        .push(format!("{path}: oneOf 命中 {matches} 个分支（应恰好 1 个）"));
                }
            }
            if let Some(not) = object.get("not") {
                let mut trial = ValidationReport::ok();
                let mut trial_seen = HashSet::new();
                validate_at(not, value, root, path, depth + 1, &mut trial_seen, &mut trial);
                if trial.valid {
                    report.errors.push(format!("{path}: 不应满足 not"));
                }
            }

            if let Some(number) = value.as_f64() {
                validate_number(object, number, path, report);
            }
            if let Some(text) = value.as_str() {
                validate_string(object, text, path, report);
            }
            if let Some(array) = value.as_array() {
                validate_array(object, array, root, path, depth, report);
            }
            if let Some(map) = value.as_object() {
                validate_object(object, map, root, path, depth, seen, report);
            }
        }
        _ => report
            .warnings
            .push(format!("{path}: 非法 Schema 节点 {schema}")),
    }
}

fn validate_number(object: &Map<String, Value>, number: f64, path: &str, report: &mut ValidationReport) {
    if let Some(min) = object.get("minimum").and_then(Value::as_f64)
        && number < min
    {
        report.errors.push(format!("{path}: 小于 minimum {min}"));
    }
    if let Some(max) = object.get("maximum").and_then(Value::as_f64)
        && number > max
    {
        report.errors.push(format!("{path}: 大于 maximum {max}"));
    }
    if let Some(min) = object.get("exclusiveMinimum") {
        match min {
            Value::Number(value) => {
                if let Some(bound) = value.as_f64()
                    && number <= bound
                {
                    report
                        .errors
                        .push(format!("{path}: 应大于 exclusiveMinimum {bound}"));
                }
            }
            Value::Bool(true) => {
                if let Some(bound) = object.get("minimum").and_then(Value::as_f64)
                    && number <= bound
                {
                    report
                        .errors
                        .push(format!("{path}: 应大于 minimum {bound}"));
                }
            }
            _ => {}
        }
    }
    if let Some(max) = object.get("exclusiveMaximum") {
        match max {
            Value::Number(value) => {
                if let Some(bound) = value.as_f64()
                    && number >= bound
                {
                    report
                        .errors
                        .push(format!("{path}: 应小于 exclusiveMaximum {bound}"));
                }
            }
            Value::Bool(true) => {
                if let Some(bound) = object.get("maximum").and_then(Value::as_f64)
                    && number >= bound
                {
                    report
                        .errors
                        .push(format!("{path}: 应小于 maximum {bound}"));
                }
            }
            _ => {}
        }
    }
    if let Some(multiple) = object.get("multipleOf").and_then(Value::as_f64)
        && multiple > 0.0
    {
        let quotient = number / multiple;
        if (quotient - quotient.round()).abs() > 1e-9 {
            report
                .errors
                .push(format!("{path}: 不是 multipleOf {multiple}"));
        }
    }
}

fn validate_string(object: &Map<String, Value>, text: &str, path: &str, report: &mut ValidationReport) {
    let length = text.chars().count() as u64;
    if let Some(min) = object.get("minLength").and_then(Value::as_u64)
        && length < min
    {
        report.errors.push(format!("{path}: 长度小于 minLength {min}"));
    }
    if let Some(max) = object.get("maxLength").and_then(Value::as_u64)
        && length > max
    {
        report.errors.push(format!("{path}: 长度大于 maxLength {max}"));
    }
    if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
        match Regex::new(pattern) {
            Ok(regex) => {
                if !regex.is_match(text) {
                    report
                        .errors
                        .push(format!("{path}: 不匹配 pattern {pattern}"));
                }
            }
            Err(_) => report
                .warnings
                .push(format!("{path}: 非法 pattern，跳过校验")),
        }
    }
    if let Some(format) = object.get("format").and_then(Value::as_str)
        && !format_matches(format, text)
    {
        // format 在多数 MCP 服务端不强制，按 warning 处理。
        report
            .warnings
            .push(format!("{path}: 不匹配 format {format}"));
    }
}

fn validate_array(
    object: &Map<String, Value>,
    array: &[Value],
    root: &Value,
    path: &str,
    depth: usize,
    report: &mut ValidationReport,
) {
    if let Some(min) = object.get("minItems").and_then(Value::as_u64)
        && (array.len() as u64) < min
    {
        report.errors.push(format!("{path}: 项数小于 minItems {min}"));
    }
    if let Some(max) = object.get("maxItems").and_then(Value::as_u64)
        && (array.len() as u64) > max
    {
        report.errors.push(format!("{path}: 项数大于 maxItems {max}"));
    }
    if object.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
        let mut seen = HashSet::new();
        for item in array {
            let key = super::contract::canonical_json(item);
            if !seen.insert(key) {
                report.errors.push(format!("{path}: uniqueItems 存在重复项"));
                break;
            }
        }
    }
    match object.get("items") {
        Some(Value::Object(_)) | Some(Value::Bool(_)) => {
            if let Some(items) = object.get("items") {
                for (index, item) in array.iter().enumerate() {
                    let mut seen = HashSet::new();
                    validate_at(
                        items,
                        item,
                        root,
                        &format!("{path}[{index}]"),
                        depth + 1,
                        &mut seen,
                        report,
                    );
                }
            }
        }
        Some(Value::Array(tuple)) => {
            for (index, item) in tuple.iter().enumerate() {
                if let Some(value) = array.get(index) {
                    let mut seen = HashSet::new();
                    validate_at(
                        item,
                        value,
                        root,
                        &format!("{path}[{index}]"),
                        depth + 1,
                        &mut seen,
                        report,
                    );
                }
            }
        }
        _ => {}
    }
}

fn validate_object(
    object: &Map<String, Value>,
    map: &Map<String, Value>,
    root: &Value,
    path: &str,
    depth: usize,
    seen: &mut HashSet<String>,
    report: &mut ValidationReport,
) {
    if let Some(required) = object.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !map.contains_key(key) {
                report
                    .errors
                    .push(format!("{path}: 缺少必填字段 {key}"));
            }
        }
    }
    if let Some(Value::Object(properties)) = object.get("properties") {
        for (key, schema) in properties {
            if let Some(value) = map.get(key) {
                validate_at(
                    schema,
                    value,
                    root,
                    &format!("{path}.{key}"),
                    depth + 1,
                    seen,
                    report,
                );
            }
        }
    }
    match object.get("additionalProperties") {
        Some(Value::Bool(false)) => {
            let known: HashSet<&String> = object
                .get("properties")
                .and_then(Value::as_object)
                .map(|map| map.keys().collect())
                .unwrap_or_default();
            for key in map.keys() {
                if !known.contains(key) {
                    report
                        .errors
                        .push(format!("{path}: 不允许额外字段 {key}"));
                }
            }
        }
        Some(Value::Object(_)) => {
            let known: HashSet<&String> = object
                .get("properties")
                .and_then(Value::as_object)
                .map(|map| map.keys().collect())
                .unwrap_or_default();
            if let Some(schema) = object.get("additionalProperties") {
                for (key, value) in map {
                    if !known.contains(key) {
                        validate_at(
                            schema,
                            value,
                            root,
                            &format!("{path}.{key}"),
                            depth + 1,
                            seen,
                            report,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn type_matches(expected: &Value, value: &Value) -> bool {
    match expected {
        Value::String(kind) => type_matches_one(kind, value),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| type_matches_one(kind, value)),
        _ => true,
    }
}

fn type_matches_one(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .map(|number| number.fract().abs() < f64::EPSILON)
            .unwrap_or(false),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn format_matches(format: &str, text: &str) -> bool {
    match format {
        "email" => text.contains('@') && text.contains('.'),
        "uri" | "url" => url::Url::parse(text).is_ok(),
        "uuid" => Regex::new(r"^[0-9a-fA-F-]{32,36}$")
            .map(|regex| regex.is_match(text))
            .unwrap_or(true),
        "date-time" => chrono::DateTime::parse_from_rfc3339(text).is_ok(),
        "date" => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
        _ => true,
    }
}

fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let path = reference.strip_prefix("#/")?;
    let mut current = root;
    for segment in path.split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        current = current.get(&segment)?;
    }
    Some(current)
}

/// 值生成器（确定性）。
pub struct ValueGenerator {
    pub include_optional: bool,
    pub max_depth: usize,
    pub seed: u64,
    pub warnings: Vec<String>,
}

impl ValueGenerator {
    pub fn new(include_optional: bool, seed: u64) -> Self {
        Self {
            include_optional,
            max_depth: 4,
            seed,
            warnings: Vec::new(),
        }
    }

    pub fn value(&mut self, schema: &Value, root: &Value, depth: usize) -> Option<Value> {
        if depth > self.max_depth {
            return None;
        }
        match schema {
            Value::Bool(true) | Value::Null => Some(Value::Null),
            Value::Bool(false) => None,
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    if reference.starts_with('#') {
                        return resolve_ref(root, reference)
                            .and_then(|target| self.value(target, root, depth + 1));
                    }
                    self.warnings
                        .push(format!("不支持远程 $ref {reference}，使用 null"));
                    return Some(Value::Null);
                }
                if let Some(value) = object.get("const") {
                    return Some(value.clone());
                }
                if let Some(Value::Array(items)) = object.get("examples")
                    && let Some(first) = items.first()
                {
                    return Some(first.clone());
                }
                if let Some(value) = object.get("example") {
                    return Some(value.clone());
                }
                if let Some(value) = object.get("default") {
                    return Some(value.clone());
                }
                if let Some(Value::Array(items)) = object.get("enum")
                    && !items.is_empty()
                {
                    let index = (self.seed as usize) % items.len();
                    return Some(items[index].clone());
                }
                if let Some(Value::Array(items)) = object.get("oneOf")
                    && let Some(first) = items.first()
                {
                    return self.value(first, root, depth + 1);
                }
                if let Some(Value::Array(items)) = object.get("anyOf") {
                    for item in items {
                        if let Some(value) = self.value(item, root, depth + 1) {
                            return Some(value);
                        }
                    }
                }
                if let Some(Value::Array(items)) = object.get("allOf") {
                    return self.value_all_of(items, root, depth);
                }
                let kind = object
                    .get("type")
                    .and_then(|value| match value {
                        Value::String(kind) => Some(kind.as_str()),
                        Value::Array(kinds) => kinds
                            .iter()
                            .filter_map(Value::as_str)
                            .find(|kind| *kind != "null"),
                        _ => None,
                    })
                    .or_else(|| {
                        if object.contains_key("properties") || object.contains_key("required") {
                            Some("object")
                        } else if object.contains_key("items") {
                            Some("array")
                        } else {
                            None
                        }
                    });
                match kind {
                    Some("object") | None => Some(self.object_value(object, root, depth)),
                    Some("array") => Some(self.array_value(object, root, depth)),
                    Some("string") => Some(Value::String(self.string_value(object))),
                    Some("integer") => Some(Value::from(self.number_value(object, true) as i64)),
                    Some("number") => Some(Value::from(self.number_value(object, false))),
                    Some("boolean") => Some(Value::Bool(true)),
                    Some("null") => Some(Value::Null),
                    Some(other) => {
                        self.warnings.push(format!("未知类型 {other}，使用 null"));
                        Some(Value::Null)
                    }
                }
            }
            _ => None,
        }
    }

    fn value_all_of(&mut self, items: &[Value], root: &Value, depth: usize) -> Option<Value> {
        let mut merged = Map::new();
        for item in items {
            if let Some(Value::Object(map)) = self.value(item, root, depth + 1) {
                for (key, value) in map {
                    merged.insert(key, value);
                }
            }
        }
        Some(Value::Object(merged))
    }

    fn object_value(&mut self, object: &Map<String, Value>, root: &Value, depth: usize) -> Value {
        let properties = object.get("properties").and_then(Value::as_object);
        let required: HashSet<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut output = Map::new();
        if let Some(properties) = properties {
            for (key, schema) in properties {
                if !self.include_optional && !required.contains(key.as_str()) {
                    continue;
                }
                if let Some(value) = self.value(schema, root, depth + 1) {
                    output.insert(key.clone(), value);
                }
            }
        }
        Value::Object(output)
    }

    fn array_value(&mut self, object: &Map<String, Value>, root: &Value, depth: usize) -> Value {
        let min = object
            .get("minItems")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .min(20) as usize;
        let length = min.max(if object.contains_key("items") { 1 } else { 0 });
        let mut items = Vec::new();
        if let Some(schema) = object.get("items") {
            for _ in 0..length {
                if let Some(value) = self.value(schema, root, depth + 1) {
                    items.push(value);
                }
            }
        }
        Value::Array(items)
    }

    fn string_value(&mut self, object: &Map<String, Value>) -> String {
        if let Some(format) = object.get("format").and_then(Value::as_str) {
            match format {
                "email" => return "test@example.com".to_string(),
                "uri" | "url" => return "https://example.com/test".to_string(),
                "uuid" => return "00000000-0000-4000-8000-000000000000".to_string(),
                "date-time" => return "2026-01-01T00:00:00Z".to_string(),
                "date" => return "2026-01-01".to_string(),
                _ => {}
            }
        }
        if object.contains_key("pattern") {
            self.warnings
                .push("pattern 字段无法可靠构造样例，使用 test".to_string());
        }
        let min = object.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
        let max = object
            .get("maxLength")
            .and_then(Value::as_u64)
            .unwrap_or(4096)
            .min(4096) as usize;
        let mut text = "test".to_string();
        if text.len() < min {
            text = "a".repeat(min.min(max));
        }
        if text.len() > max {
            text.truncate(max);
        }
        text
    }

    fn number_value(&mut self, object: &Map<String, Value>, integer: bool) -> f64 {
        let min = object.get("minimum").and_then(Value::as_f64).unwrap_or(0.0);
        let max = object
            .get("maximum")
            .and_then(Value::as_f64)
            .unwrap_or(min + 100.0);
        let mut value = min.clamp(0.0, max);
        if object.get("exclusiveMinimum").and_then(Value::as_f64) == Some(value) {
            value += 1.0;
        }
        if let Some(multiple) = object.get("multipleOf").and_then(Value::as_f64)
            && multiple > 0.0
        {
            value = (value / multiple).ceil() * multiple;
        }
        if integer {
            value = value.round();
        }
        value.clamp(min, max)
    }
}

/// 生成一个正向参数值；失败时返回空对象并记录 warning。
pub fn generate_positive(schema: &Value, seed: u64) -> (Value, Vec<String>) {
    let mut generator = ValueGenerator::new(false, seed);
    let value = generator
        .value(schema, schema, 0)
        .unwrap_or_else(|| json!({}));
    (value, generator.warnings)
}

/// 生成包含全部可选字段的参数值。
pub fn generate_with_optional(schema: &Value, seed: u64) -> Option<(Value, Vec<String>)> {
    let mut generator = ValueGenerator::new(true, seed);
    let value = generator.value(schema, schema, 0)?;
    Some((value, generator.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_common_keywords() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string", "minLength": 2},
                "age": {"type": "integer", "minimum": 0, "maximum": 120},
                "tag": {"enum": ["a", "b"]}
            },
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({"name": "ok", "age": 1})).valid);
        assert!(!validate(&schema, &json!({"name": "o", "age": 1})).valid);
        assert!(!validate(&schema, &json!({"name": "ok", "age": 121})).valid);
        assert!(!validate(&schema, &json!({"name": "ok", "age": 1, "x": 1})).valid);
        assert!(!validate(&schema, &json!({"name": "ok", "age": 1, "tag": "c"})).valid);
    }

    #[test]
    fn resolves_local_ref_and_composition() {
        let schema = json!({
            "definitions": {"name": {"type": "string", "minLength": 1}},
            "type": "object",
            "properties": {"name": {"$ref": "#/definitions/name"}},
            "required": ["name"]
        });
        assert!(validate(&schema, &json!({"name": "x"})).valid);
        assert!(!validate(&schema, &json!({"name": ""})).valid);
    }

    #[test]
    fn generates_required_only_and_optional() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer", "minimum": 2, "maximum": 10}
            }
        });
        let (value, _) = generate_positive(&schema, 1);
        assert_eq!(value, json!({"name": "test"}));
        let (value, _) = generate_with_optional(&schema, 1).expect("生成失败");
        assert_eq!(value["count"], 2);
    }

    #[test]
    fn format_and_pattern_are_best_effort() {
        let schema = json!({"type": "string", "format": "email", "pattern": "^.+@.+$"});
        let (value, _) = generate_positive(&schema, 1);
        assert!(validate(&schema, &value).valid);
    }
}
