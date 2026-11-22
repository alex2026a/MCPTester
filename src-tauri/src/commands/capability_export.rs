//! 能力快照导出（v0.6.1）
//!
//! P1 提供接口清单 CSV 与能力快照 JSON；P2 增加参数明细 CSV。
//! CSV 使用 UTF-8 BOM、CRLF、RFC 4180 转义与防公式注入；不包含请求头/凭据。

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tauri::State;

use crate::commands::AppState;
use crate::mcp::models::{
    CapabilitySnapshot, ConnectionState, PromptInfo, ResourceInfo, ResourceTemplateInfo, ToolInfo,
    TransportKind,
};
use crate::storage::server_repo;
use crate::testing::contract::capability_hash;

const FORMAT_CSV: &str = "csv";
const FORMAT_CSV_PARAMS: &str = "csv_params";
const FORMAT_JSON: &str = "json";

const CSV_BOM: char = '\u{feff}';
const CSV_MAX_TEXT: usize = 8000;

const CSV_COLUMNS: [&str; 24] = [
    "server_name",
    "server_id",
    "transport",
    "protocol_version",
    "capability_hash",
    "fetched_at",
    "stale",
    "category",
    "method",
    "interface_id",
    "name",
    "title",
    "description",
    "uri",
    "param_count",
    "required_params",
    "params",
    "header_params",
    "annotations",
    "mime_type",
    "size_bytes",
    "output_schema_present",
    "input_schema",
    "output_schema",
];

const PARAM_CSV_COLUMNS: [&str; 18] = [
    "server_name",
    "server_id",
    "category",
    "interface_id",
    "interface_name",
    "method",
    "param_source",
    "param_path",
    "param_name",
    "param_type",
    "required",
    "default",
    "enum",
    "format",
    "constraints",
    "description",
    "header_name",
    "schema_ref",
];

#[derive(Debug, Clone)]
struct ExportMeta {
    server_id: String,
    server_name: String,
    transport: TransportKind,
    protocol_version: Option<String>,
    capability_hash: String,
    fetched_at: DateTime<Utc>,
    stale: bool,
}

struct CsvRecord {
    sort_category: u8,
    sort_name: String,
    fields: Vec<String>,
}

/// 预览接口清单 CSV / 能力快照 JSON。
#[tauri::command]
pub async fn preview_capability_export(
    state: State<'_, AppState>,
    id: String,
    format: String,
    include_schema: Option<bool>,
) -> Result<String, String> {
    let (snapshot, meta) = collect_export(&state, &id).await?;
    build_export(
        &snapshot,
        &meta,
        &format,
        include_schema.unwrap_or(false),
    )
}

/// 导出接口清单 CSV / 能力快照 JSON 到文件，返回路径。
#[tauri::command]
pub async fn export_capability_export(
    state: State<'_, AppState>,
    id: String,
    format: String,
    path: Option<String>,
    include_schema: Option<bool>,
) -> Result<String, String> {
    let (snapshot, meta) = collect_export(&state, &id).await?;
    let payload = build_export(
        &snapshot,
        &meta,
        &format,
        include_schema.unwrap_or(false),
    )?;
    let target = match path {
        Some(path) => PathBuf::from(path),
        None => state.data_dir.join(default_filename(&meta, &format)),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建导出目录失败：{e}"))?;
    }
    std::fs::write(&target, payload)
        .map_err(|e| format!("写入导出文件失败：{e}"))?;
    Ok(target.display().to_string())
}

async fn collect_export(
    state: &AppState,
    id: &str,
) -> Result<(CapabilitySnapshot, ExportMeta), String> {
    let handle = state
        .manager
        .handle(id)
        .await
        .ok_or_else(|| "服务器未连接，请先连接".to_string())?;
    let snapshot = handle
        .capabilities(false)
        .await
        .map_err(|e| e.message())?;
    let protocol_version = match handle.state().await {
        ConnectionState::Ready { handshake } => Some(handshake.protocol_version),
        _ => None,
    };
    let transport = handle.transport_kind().await;
    let server_name = state
        .db
        .with_read_blocking(|conn| server_repo::get(conn, id))
        .ok()
        .flatten()
        .map(|config| config.name)
        .unwrap_or_else(|| handle.server_name.clone());
    let capability_hash = capability_hash(&snapshot);
    Ok((
        snapshot.clone(),
        ExportMeta {
            server_id: id.to_string(),
            server_name,
            transport,
            protocol_version,
            capability_hash,
            fetched_at: snapshot.fetched_at,
            stale: snapshot.stale,
        },
    ))
}

fn build_export(
    snapshot: &CapabilitySnapshot,
    meta: &ExportMeta,
    format: &str,
    include_schema: bool,
) -> Result<String, String> {
    match format {
        FORMAT_CSV => Ok(build_csv(snapshot, meta, include_schema)),
        FORMAT_CSV_PARAMS => Ok(build_csv_params(snapshot, meta)),
        FORMAT_JSON => build_json(snapshot, meta),
        other => Err(format!("不支持的能力导出格式：{other}")),
    }
}

fn build_csv(
    snapshot: &CapabilitySnapshot,
    meta: &ExportMeta,
    include_schema: bool,
) -> String {
    let mut records = Vec::new();
    let header_params_enabled = supports_header_params(meta);

    for tool in &snapshot.tools {
        records.push(tool_record(
            tool,
            meta,
            include_schema,
            header_params_enabled,
        ));
    }
    for resource in &snapshot.resources {
        records.push(resource_record(resource, meta));
    }
    for template in &snapshot.resource_templates {
        records.push(template_record(template, meta));
    }
    for prompt in &snapshot.prompts {
        records.push(prompt_record(prompt, meta));
    }
    records.sort_by(|a, b| {
        a.sort_category
            .cmp(&b.sort_category)
            .then_with(|| a.sort_name.cmp(&b.sort_name))
    });

    let mut out = String::new();
    out.push(CSV_BOM);
    out.push_str(&csv_row(
        &CSV_COLUMNS
            .iter()
            .map(|column| csv_field(column))
            .collect::<Vec<_>>(),
    ));
    for record in records {
        out.push_str(&csv_row(&record.fields));
    }
    out
}

fn build_csv_params(snapshot: &CapabilitySnapshot, meta: &ExportMeta) -> String {
    let mut rows = Vec::new();
    for tool in &snapshot.tools {
        rows.extend(tool_param_rows(tool, meta));
    }
    for prompt in &snapshot.prompts {
        let interface_id = format!("prompt:{}", prompt.name);
        for argument in &prompt.arguments {
            rows.push(ParamRow {
                sort_category: 3,
                interface_id: interface_id.clone(),
                interface_name: prompt.name.clone(),
                category: "prompt",
                method: "prompts/get",
                source: "prompt_argument",
                path: argument.name.clone(),
                name: argument.name.clone(),
                ty: "string".to_string(),
                required: argument.required,
                default: String::new(),
                enum_values: String::new(),
                format: String::new(),
                constraints: String::new(),
                description: argument.description.clone().unwrap_or_default(),
                header_name: String::new(),
                schema_ref: String::new(),
            });
        }
    }
    for template in &snapshot.resource_templates {
        let interface_id = format!("resource_template:{}", template.uri_template);
        for name in uri_template_variables(&template.uri_template) {
            rows.push(ParamRow {
                sort_category: 2,
                interface_id: interface_id.clone(),
                interface_name: template.name.clone(),
                category: "resource_template",
                method: "resources/read",
                source: "uri_template",
                path: name.clone(),
                name,
                ty: "string".to_string(),
                required: true,
                default: String::new(),
                enum_values: String::new(),
                format: String::new(),
                constraints: String::new(),
                description: String::new(),
                header_name: String::new(),
                schema_ref: String::new(),
            });
        }
    }
    rows.sort_by(|a, b| {
        a.sort_category
            .cmp(&b.sort_category)
            .then_with(|| a.interface_id.cmp(&b.interface_id))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut out = String::new();
    out.push(CSV_BOM);
    out.push_str(&csv_row(
        &PARAM_CSV_COLUMNS
            .iter()
            .map(|column| csv_field(column))
            .collect::<Vec<_>>(),
    ));
    for row in rows {
        out.push_str(&csv_row(&param_row_fields(&row, meta)));
    }
    out
}

#[derive(Debug, Clone)]
struct ParamRow {
    sort_category: u8,
    interface_id: String,
    interface_name: String,
    category: &'static str,
    method: &'static str,
    source: &'static str,
    path: String,
    name: String,
    ty: String,
    required: bool,
    default: String,
    enum_values: String,
    format: String,
    constraints: String,
    description: String,
    header_name: String,
    schema_ref: String,
}

fn param_row_fields(row: &ParamRow, meta: &ExportMeta) -> Vec<String> {
    vec![
        text_cell(&meta.server_name),
        text_cell(&meta.server_id),
        text_cell(row.category),
        text_cell(&row.interface_id),
        text_cell(&row.interface_name),
        text_cell(row.method),
        text_cell(row.source),
        text_cell(&row.path),
        text_cell(&row.name),
        text_cell(&row.ty),
        raw_cell(if row.required { "true" } else { "false" }),
        text_cell(&row.default),
        text_cell(&row.enum_values),
        text_cell(&row.format),
        text_cell(&row.constraints),
        text_cell(&row.description),
        text_cell(&row.header_name),
        text_cell(&row.schema_ref),
    ]
}

fn tool_param_rows(tool: &ToolInfo, meta: &ExportMeta) -> Vec<ParamRow> {
    let Some(schema) = tool.input_schema.as_ref() else {
        return Vec::new();
    };
    let mut ctx = ParamCtx {
        root: schema,
        interface_id: format!("tool:{}", tool.name),
        interface_name: tool.name.clone(),
        header_enabled: supports_header_params(meta),
        seen_headers: std::collections::HashSet::new(),
        seen_rows: std::collections::HashSet::new(),
        rows: Vec::new(),
    };
    let mut ref_stack = Vec::new();
    let root_holder = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_local_ref(schema, reference));
    let effective_root = root_holder.as_ref().unwrap_or(schema);
    if let Some(properties) = effective_root.get("properties").and_then(Value::as_object) {
        let required = required_set(effective_root);
        for (name, child) in properties {
            ctx.walk(
                child,
                name,
                name,
                required.contains(name),
                0,
                true,
                &mut ref_stack,
            );
        }
    }
    ctx.rows
}

struct ParamCtx<'a> {
    root: &'a Value,
    interface_id: String,
    interface_name: String,
    header_enabled: bool,
    seen_headers: std::collections::HashSet<String>,
    seen_rows: std::collections::HashSet<(String, String, String)>,
    rows: Vec<ParamRow>,
}

impl ParamCtx<'_> {
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        schema: &Value,
        path: &str,
        name: &str,
        required: bool,
        depth: usize,
        root_property: bool,
        ref_stack: &mut Vec<String>,
    ) {
        if depth > 8 {
            return;
        }
        let mut schema_ref = None;
        let resolved = if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            schema_ref = Some(reference.to_string());
            if ref_stack.iter().any(|item| item == reference) {
                self.push_row(
                    schema,
                    path,
                    name,
                    required,
                    root_property,
                    schema_ref,
                    true,
                );
                return;
            }
            match resolve_local_ref(self.root, reference) {
                Some(value) => {
                    ref_stack.push(reference.to_string());
                    Some(value)
                }
                None => {
                    self.push_row(
                        schema,
                        path,
                        name,
                        required,
                        root_property,
                        schema_ref,
                        true,
                    );
                    return;
                }
            }
        } else {
            None
        };
        let effective = resolved.as_ref().unwrap_or(schema);

        self.push_row(
            effective,
            path,
            name,
            required,
            root_property,
            schema_ref.clone(),
            false,
        );

        if depth < 8 {
            self.walk_children(effective, path, depth, ref_stack);
        }
        if schema_ref.is_some() {
            ref_stack.pop();
        }
    }

    fn walk_children(
        &mut self,
        schema: &Value,
        path: &str,
        depth: usize,
        ref_stack: &mut Vec<String>,
    ) {
        let mut pushed = false;
        let resolved = if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if ref_stack.iter().any(|item| item == reference) {
                return;
            }
            match resolve_local_ref(self.root, reference) {
                Some(value) => {
                    ref_stack.push(reference.to_string());
                    pushed = true;
                    Some(value)
                }
                None => return,
            }
        } else {
            None
        };
        let effective = resolved.as_ref().unwrap_or(schema);

        if let Some(properties) = effective.get("properties").and_then(Value::as_object) {
            let required = required_set(effective);
            for (name, child) in properties {
                self.walk(
                    child,
                    &format!("{path}.{name}"),
                    name,
                    required.contains(name),
                    depth + 1,
                    false,
                    ref_stack,
                );
            }
        }
        if let Some(items) = effective.get("items") {
            self.walk_children(items, &format!("{path}[]"), depth + 1, ref_stack);
        }
        if let Some(branches) = effective.get("allOf").and_then(Value::as_array) {
            for branch in branches {
                self.walk_children(branch, path, depth + 1, ref_stack);
            }
        }
        for keyword in ["oneOf", "anyOf"] {
            if let Some(branches) = effective.get(keyword).and_then(Value::as_array) {
                for branch in branches {
                    let branch_schema = branch
                        .get("$ref")
                        .and_then(Value::as_str)
                        .and_then(|reference| resolve_local_ref(self.root, reference))
                        .unwrap_or_else(|| branch.clone());
                    if branch_schema.get("properties").is_some() {
                        self.walk_children(&branch_schema, path, depth + 1, ref_stack);
                        break;
                    }
                }
            }
        }
        if let Some(additional) = effective.get("additionalProperties") {
            match additional {
                Value::Bool(true) => {
                    self.push_row(
                        &Value::Bool(true),
                        &format!("{path}.*"),
                        "*",
                        false,
                        false,
                        None,
                        true,
                    );
                }
                Value::Object(_) => {
                    self.walk(
                        additional,
                        &format!("{path}.*"),
                        "*",
                        false,
                        depth + 1,
                        false,
                        ref_stack,
                    );
                }
                _ => {}
            }
        }
        if pushed {
            ref_stack.pop();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_row(
        &mut self,
        schema: &Value,
        path: &str,
        name: &str,
        required: bool,
        root_property: bool,
        schema_ref: Option<String>,
        fallback_any: bool,
    ) {
        let ty = if fallback_any {
            "any".to_string()
        } else {
            schema_type(schema)
        };
        let key = (path.to_string(), name.to_string(), ty.clone());
        if !self.seen_rows.insert(key) {
            return;
        }
        let header_name = if self.header_enabled
            && root_property
            && schema_ref.is_none()
            && schema.get("$ref").is_none()
            && matches!(
                schema.get("type").and_then(Value::as_str),
                Some("string" | "integer" | "boolean")
            )
        {
            schema
                .get("x-mcp-header")
                .and_then(Value::as_str)
                .filter(|header| is_http_token(header))
                .and_then(|header| {
                    if self.seen_headers.insert(header.to_ascii_lowercase()) {
                        Some(format!("Mcp-Param-{header}"))
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        self.rows.push(ParamRow {
            sort_category: 0,
            interface_id: self.interface_id.clone(),
            interface_name: self.interface_name.clone(),
            category: "tool",
            method: "tools/call",
            source: "input_schema",
            path: path.to_string(),
            name: name.to_string(),
            ty,
            required,
            default: schema
                .get("default")
                .map(|value| serde_json::to_string(value).unwrap_or_default())
                .unwrap_or_default(),
            enum_values: schema
                .get("enum")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .unwrap_or_default(),
            format: schema
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            constraints: constraints_json(schema),
            description: schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            header_name,
            schema_ref: schema_ref.unwrap_or_default(),
        });
    }
}

fn required_set(schema: &Value) -> std::collections::HashSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_local_ref(root: &Value, reference: &str) -> Option<Value> {
    let path = reference.strip_prefix("#/")?;
    let mut current = root;
    for segment in path.split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        current = current.get(segment.as_str())?;
    }
    Some(current.clone())
}

fn constraints_json(schema: &Value) -> String {
    let mut object = serde_json::Map::new();
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "multipleOf",
    ] {
        if let Some(value) = schema.get(key) {
            object.insert(key.to_string(), value.clone());
        }
    }
    if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array) {
        object.insert("prefix_items".to_string(), json!(prefix.len()));
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            let variants: Vec<String> = branches.iter().map(schema_type).collect();
            object.insert("variants".to_string(), json!(variants));
            break;
        }
    }
    if let Some(additional) = schema.get("additionalProperties") {
        object.insert("additional_properties".to_string(), additional.clone());
    }
    if object.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&object).unwrap_or_default()
    }
}

fn tool_record(
    tool: &ToolInfo,
    meta: &ExportMeta,
    include_schema: bool,
    header_params_enabled: bool,
) -> CsvRecord {
    let top_level = top_level_params(tool.input_schema.as_ref());
    let header_params = if header_params_enabled {
        tool_header_params(tool.input_schema.as_ref())
    } else {
        Vec::new()
    };
    CsvRecord {
        sort_category: 0,
        sort_name: tool.name.clone(),
        fields: base_fields(
            meta,
            "tool",
            "tools/call",
            format!("tool:{}", tool.name),
            &tool.name,
            tool.title.as_deref(),
            tool.description.as_deref(),
            "",
            &top_level,
            &header_params,
            tool.annotations.as_ref(),
            None,
            None,
            Some(tool.output_schema.is_some()),
            tool.input_schema.as_ref(),
            tool.output_schema.as_ref(),
            include_schema,
        ),
    }
}

fn resource_record(resource: &ResourceInfo, meta: &ExportMeta) -> CsvRecord {
    let params: Vec<TopLevelParam> = Vec::new();
    CsvRecord {
        sort_category: 1,
        sort_name: resource.uri.clone(),
        fields: base_fields(
            meta,
            "resource",
            "resources/read",
            format!("resource:{}", resource.uri),
            &resource.name,
            resource.title.as_deref(),
            resource.description.as_deref(),
            &resource.uri,
            &params,
            &[],
            resource.annotations.as_ref(),
            resource.mime_type.as_deref(),
            resource.size,
            None,
            None,
            None,
            false,
        ),
    }
}

fn template_record(template: &ResourceTemplateInfo, meta: &ExportMeta) -> CsvRecord {
    let names = uri_template_variables(&template.uri_template);
    let params: Vec<TopLevelParam> = names
        .into_iter()
        .map(|name| TopLevelParam {
            name,
            ty: "string".to_string(),
            required: true,
        })
        .collect();
    CsvRecord {
        sort_category: 2,
        sort_name: template.uri_template.clone(),
        fields: base_fields(
            meta,
            "resource_template",
            "resources/read",
            format!("resource_template:{}", template.uri_template),
            &template.name,
            template.title.as_deref(),
            template.description.as_deref(),
            &template.uri_template,
            &params,
            &[],
            None,
            template.mime_type.as_deref(),
            None,
            None,
            None,
            None,
            false,
        ),
    }
}

fn prompt_record(prompt: &PromptInfo, meta: &ExportMeta) -> CsvRecord {
    let params: Vec<TopLevelParam> = prompt
        .arguments
        .iter()
        .map(|arg| TopLevelParam {
            name: arg.name.clone(),
            ty: "string".to_string(),
            required: arg.required,
        })
        .collect();
    CsvRecord {
        sort_category: 3,
        sort_name: prompt.name.clone(),
        fields: base_fields(
            meta,
            "prompt",
            "prompts/get",
            format!("prompt:{}", prompt.name),
            &prompt.name,
            prompt.title.as_deref(),
            prompt.description.as_deref(),
            "",
            &params,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn base_fields(
    meta: &ExportMeta,
    category: &str,
    method: &str,
    interface_id: String,
    name: &str,
    title: Option<&str>,
    description: Option<&str>,
    uri: &str,
    params: &[TopLevelParam],
    header_params: &[(String, String)],
    annotations: Option<&Value>,
    mime_type: Option<&str>,
    size_bytes: Option<u64>,
    output_schema_present: Option<bool>,
    input_schema: Option<&Value>,
    output_schema: Option<&Value>,
    include_schema: bool,
) -> Vec<String> {
    let required = params.iter().filter(|param| param.required).count();
    vec![
        text_cell(&meta.server_name),
        text_cell(&meta.server_id),
        text_cell(meta.transport.as_str()),
        text_cell(meta.protocol_version.as_deref().unwrap_or("")),
        text_cell(&meta.capability_hash),
        text_cell(&meta.fetched_at.to_rfc3339()),
        raw_cell(if meta.stale { "true" } else { "false" }),
        text_cell(category),
        text_cell(method),
        text_cell(&interface_id),
        text_cell(name),
        text_cell(title.unwrap_or("")),
        text_cell(description.unwrap_or("")),
        text_cell(uri),
        raw_cell(&params.len().to_string()),
        raw_cell(&required.to_string()),
        text_cell(&params_summary(params)),
        text_cell(
            &header_params
                .iter()
                .map(|(argument, header)| format!("{argument} -> {header}"))
                .collect::<Vec<_>>()
                .join("; "),
        ),
        text_cell(
            &annotations
                .map(|value| serde_json::to_string(value).unwrap_or_default())
                .unwrap_or_default(),
        ),
        text_cell(mime_type.unwrap_or("")),
        raw_cell(&size_bytes.map(|value| value.to_string()).unwrap_or_default()),
        raw_cell(
            output_schema_present
                .map(|value| if value { "true" } else { "false" })
                .unwrap_or(""),
        ),
        json_cell(input_schema, include_schema),
        json_cell(output_schema, include_schema),
    ]
}

#[derive(Debug, Clone)]
struct TopLevelParam {
    name: String,
    ty: String,
    required: bool,
}

fn top_level_params(schema: Option<&Value>) -> Vec<TopLevelParam> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    properties
        .iter()
        .map(|(name, value)| TopLevelParam {
            name: name.clone(),
            ty: schema_type(value),
            required: required.contains(&name.as_str()),
        })
        .collect()
}

fn schema_type(schema: &Value) -> String {
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if ty == "array" {
            let item = schema
                .get("items")
                .map(schema_type)
                .unwrap_or_else(|| "any".to_string());
            return format!("array<{item}>");
        }
        return ty.to_string();
    }
    if schema.get("properties").is_some() {
        return "object".to_string();
    }
    if schema.get("items").is_some() {
        return "array".to_string();
    }
    if let Some(items) = schema.get("enum").and_then(Value::as_array)
        && let Some(first) = items.first()
    {
        return match first {
            Value::String(_) => "string".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Number(_) => "number".to_string(),
            _ => "any".to_string(),
        };
    }
    "any".to_string()
}

fn params_summary(params: &[TopLevelParam]) -> String {
    params
        .iter()
        .map(|param| {
            format!(
                "{}:{}{}",
                param.name,
                param.ty,
                if param.required { "*" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn supports_header_params(meta: &ExportMeta) -> bool {
    meta.transport == TransportKind::StreamableHttp
        && meta
            .protocol_version
            .as_deref()
            .is_some_and(|version| version >= "2026-07-28")
}

fn tool_header_params(schema: Option<&Value>) -> Vec<(String, String)> {
    let Some(properties) = schema
        .and_then(|value| value.get("properties"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for (name, value) in properties {
        if value.get("$ref").is_some() {
            continue;
        }
        let Some(header) = value.get("x-mcp-header").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(
            value.get("type").and_then(Value::as_str),
            Some("string" | "integer" | "boolean")
        ) {
            continue;
        }
        if !is_http_token(header) || !seen.insert(header.to_ascii_lowercase()) {
            continue;
        }
        result.push((name.clone(), format!("Mcp-Param-{header}")));
    }
    result
}

fn is_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// 提取 URI 模板变量（简单 `{var}` 与 RFC 6570 `{?a,b}` / `{&a}` 形式）。
fn uri_template_variables(template: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let expression = &after[..end];
        let expression = expression
            .strip_prefix(['+', '#', '.', '/', ';', '?', '&'])
            .unwrap_or(expression);
        for item in expression.split(',') {
            let name = item
                .split('*')
                .next()
                .unwrap_or(item)
                .split(':')
                .next()
                .unwrap_or(item)
                .trim();
            if !name.is_empty() && seen.insert(name.to_string()) {
                result.push(name.to_string());
            }
        }
        rest = &after[end + 1..];
    }
    result
}

fn json_cell(value: Option<&Value>, include_schema: bool) -> String {
    if !include_schema {
        return text_cell("");
    }
    let text = value
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .unwrap_or_default();
    text_cell(&truncate_text(&text, CSV_MAX_TEXT))
}

fn text_cell(value: &str) -> String {
    csv_field(&neutralize_formula(&normalize_text(value)))
}

fn raw_cell(value: &str) -> String {
    csv_field(value)
}

fn normalize_text(value: &str) -> String {
    value
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut text: String = value.chars().take(max_chars).collect();
    text.push_str("…（已截断）");
    text
}

fn neutralize_formula(value: &str) -> String {
    let trimmed = value.trim_start();
    if trimmed
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@'))
        && trimmed.parse::<f64>().is_err()
    {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn csv_field(value: &str) -> String {
    if value.contains(',')
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r')
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_row(fields: &[String]) -> String {
    let mut row = fields.join(",");
    row.push_str("\r\n");
    row
}

fn default_filename(meta: &ExportMeta, format: &str) -> String {
    let slug = sanitize_slug(&meta.server_name);
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    match format {
        FORMAT_CSV => format!("mcptester-interfaces-{slug}-{stamp}.csv"),
        FORMAT_CSV_PARAMS => format!("mcptester-interface-params-{slug}-{stamp}.csv"),
        FORMAT_JSON => format!("mcptester-capabilities-{slug}-{stamp}.json"),
        other => format!("mcptester-export-{slug}-{stamp}.{other}"),
    }
}

fn sanitize_slug(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|ch| {
            if ch.is_whitespace() || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '-'
            } else {
                ch
            }
        })
        .collect();
    slug = slug
        .trim_matches('-')
        .chars()
        .take(40)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "server".to_string()
    } else {
        slug
    }
}

fn build_json(snapshot: &CapabilitySnapshot, meta: &ExportMeta) -> Result<String, String> {
    let value = json!({
        "exported_by": format!("MCP Tester v{}", env!("CARGO_PKG_VERSION")),
        "exported_at": Utc::now().to_rfc3339(),
        "server": {
            "id": &meta.server_id,
            "name": &meta.server_name,
            "transport": meta.transport.as_str(),
            "protocol_version": &meta.protocol_version,
            "capability_hash": &meta.capability_hash,
            "fetched_at": meta.fetched_at.to_rfc3339(),
            "stale": meta.stale,
        },
        "snapshot": snapshot,
    });
    serde_json::to_string_pretty(&value).map_err(|e| format!("序列化能力快照失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{
        PromptArgument, PromptInfo, ResourceInfo, ResourceTemplateInfo, ToolInfo,
    };
    use chrono::Utc;
    use serde_json::json;

    fn meta(transport: TransportKind, protocol: Option<&str>) -> ExportMeta {
        ExportMeta {
            server_id: "srv-1".to_string(),
            server_name: "示例服务器".to_string(),
            transport,
            protocol_version: protocol.map(str::to_string),
            capability_hash: "hash".to_string(),
            fetched_at: Utc::now(),
            stale: false,
        }
    }

    fn snapshot() -> CapabilitySnapshot {
        CapabilitySnapshot {
            server_id: "srv-1".to_string(),
            tools: vec![ToolInfo {
                name: "search".to_string(),
                title: Some("搜索".to_string()),
                description: Some("描述, 含逗号".to_string()),
                input_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "关键词" },
                        "limit": { "type": "integer" },
                        "region": { "type": "string", "x-mcp-header": "Region" },
                        "bad": { "type": "number", "x-mcp-header": "Bad" },
                        "nested": { "type": "object", "properties": { "x": { "type": "string" } } }
                    },
                    "required": ["query"]
                })),
                output_schema: Some(json!({ "type": "object" })),
                annotations: Some(json!({ "readOnlyHint": true })),
            }],
            resources: vec![ResourceInfo {
                uri: "mock://hello".to_string(),
                name: "hello".to_string(),
                title: None,
                description: None,
                mime_type: Some("text/plain".to_string()),
                size: Some(12),
                annotations: None,
            }],
            resource_templates: vec![ResourceTemplateInfo {
                uri_template: "mock://item/{id}{?limit}".to_string(),
                name: "item".to_string(),
                title: None,
                description: None,
                mime_type: None,
            }],
            prompts: vec![PromptInfo {
                name: "greet".to_string(),
                title: None,
                description: None,
                arguments: vec![PromptArgument {
                    name: "who".to_string(),
                    description: None,
                    required: true,
                }],
            }],
            fetched_at: Utc::now(),
            stale: false,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn csv_has_bom_header_and_four_categories() {
        let text = build_csv(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            false,
        );
        assert!(text.starts_with(CSV_BOM));
        assert!(text.contains("server_name,server_id,transport"));
        let rows: Vec<&str> = text.trim_start_matches(CSV_BOM).split("\r\n").collect();
        // 表头 + 4 类 + 末尾空行
        assert_eq!(rows.iter().filter(|row| !row.is_empty()).count(), 5);
        assert!(text.contains("tool:search"));
        assert!(text.contains("resource:mock://hello"));
        assert!(text.contains("resource_template:mock://item/{id}{?limit}"));
        assert!(text.contains("prompt:greet"));
    }

    #[test]
    fn csv_escapes_commas_quotes_and_formula() {
        let mut value = snapshot();
        value.tools[0].name = "=cmd".to_string();
        value.tools[0].description = Some("say \"hi\", ok".to_string());
        let text = build_csv(
            &value,
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            false,
        );
        assert!(text.contains("'=cmd"));
        assert!(text.contains("\"say \"\"hi\"\", ok\""));
    }

    #[test]
    fn header_params_require_modern_http_and_primitive() {
        let modern = build_csv(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            false,
        );
        assert!(modern.contains("region -> Mcp-Param-Region"));
        assert!(!modern.contains("bad -> Mcp-Param-Bad"));

        let legacy = build_csv(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2025-11-25")),
            false,
        );
        assert!(!legacy.contains("Mcp-Param-Region"));

        let stdio = build_csv(&snapshot(), &meta(TransportKind::Stdio, None), false);
        assert!(!stdio.contains("Mcp-Param-Region"));
    }

    #[test]
    fn include_schema_toggle_and_truncation() {
        let without = build_csv(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            false,
        );
        assert!(!without.contains("\"properties\""));
        let with = build_csv(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            true,
        );
        assert!(with.contains("properties"));

        let long = json!({ "description": "x".repeat(CSV_MAX_TEXT + 10) });
        let cell = json_cell(Some(&long), true);
        assert!(cell.contains("已截断"));
        assert!(!cell.trim_matches('"').contains("\"description\":\"xxxx"));
    }

    #[test]
    fn empty_snapshot_has_header_only() {
        let empty = CapabilitySnapshot {
            server_id: "srv-1".to_string(),
            tools: Vec::new(),
            resources: Vec::new(),
            resource_templates: Vec::new(),
            prompts: Vec::new(),
            fetched_at: Utc::now(),
            stale: false,
            warnings: Vec::new(),
        };
        let text = build_csv(&empty, &meta(TransportKind::Stdio, None), false);
        let rows: Vec<&str> = text.trim_start_matches(CSV_BOM).split("\r\n").collect();
        assert_eq!(rows.iter().filter(|row| !row.is_empty()).count(), 1);
    }

    #[test]
    fn uri_template_variables_support_operator_forms() {
        assert_eq!(
            uri_template_variables("mock://item/{id}{?limit,cursor}{+path}{var:3}{var*}"),
            vec!["id", "limit", "cursor", "path", "var"]
        );
    }

    #[test]
    fn json_snapshot_has_meta_and_deserializes() {
        let snap = snapshot();
        let text = build_json(
            &snap,
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
        )
        .expect("json");
        let value: Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(value["exported_by"], format!("MCP Tester v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(value["server"]["id"], "srv-1");
        let parsed: CapabilitySnapshot =
            serde_json::from_value(value["snapshot"].clone()).expect("snapshot");
        assert_eq!(parsed.tools[0].name, "search");
    }

    #[test]
    fn csv_params_format_is_supported() {
        let snap = snapshot();
        let result = build_export(
            &snap,
            &meta(TransportKind::Stdio, None),
            "csv_params",
            false,
        );
        assert!(result.is_ok(), "P2 应支持 csv_params");
    }

    #[test]
    fn csv_params_covers_tool_prompt_template() {
        let text = build_export(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            "csv_params",
            false,
        )
        .expect("csv_params");
        assert!(text.contains("param_source,param_path"));
        assert!(text.contains("input_schema,query"));
        assert!(text.contains("input_schema,nested"));
        assert!(text.contains("input_schema,nested.x"));
        assert!(text.contains("prompt_argument,who"));
        assert!(text.contains("uri_template,id"));
        assert!(text.contains("uri_template,limit"));
        assert!(text.contains("Mcp-Param-Region"));
        assert!(!text.contains("Mcp-Param-Bad"));
    }

    #[test]
    fn csv_params_expands_sibling_refs_and_stops_cycles() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "child": {
                    "type": "object",
                    "properties": { "value": { "type": "string" } }
                },
                "a": {
                    "type": "object",
                    "properties": { "b": { "$ref": "#/$defs/b" } }
                },
                "b": {
                    "type": "object",
                    "properties": { "a": { "$ref": "#/$defs/a" } }
                }
            },
            "properties": {
                "left": { "$ref": "#/$defs/child" },
                "right": { "$ref": "#/$defs/child" }
            }
        });
        let tool = ToolInfo {
            name: "refs".to_string(),
            title: None,
            description: None,
            input_schema: Some(schema),
            output_schema: None,
            annotations: None,
        };
        let rows = tool_param_rows(
            &tool,
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
        );
        assert!(rows.iter().any(|row| row.path == "left.value"));
        assert!(rows.iter().any(|row| row.path == "right.value"));

        let cycle = json!({
            "type": "object",
            "$defs": {
                "a": { "type": "object", "properties": { "b": { "$ref": "#/$defs/b" } } },
                "b": { "type": "object", "properties": { "a": { "$ref": "#/$defs/a" } } }
            },
            "properties": { "root": { "$ref": "#/$defs/a" } }
        });
        let cycle_tool = ToolInfo {
            name: "cycle".to_string(),
            title: None,
            description: None,
            input_schema: Some(cycle),
            output_schema: None,
            annotations: None,
        };
        let cycle_rows = tool_param_rows(
            &cycle_tool,
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
        );
        assert!(!cycle_rows.is_empty());
        assert!(cycle_rows.iter().any(|row| !row.schema_ref.is_empty()));
    }

    #[test]
    fn csv_params_header_name_requires_modern_tool_root_primitive() {
        let modern = build_export(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2026-07-28")),
            "csv_params",
            false,
        )
        .expect("csv_params");
        assert!(modern.contains("Mcp-Param-Region"));
        assert!(!modern.contains("Mcp-Param-Bad"));

        let legacy = build_export(
            &snapshot(),
            &meta(TransportKind::StreamableHttp, Some("2025-11-25")),
            "csv_params",
            false,
        )
        .expect("csv_params");
        assert!(!legacy.contains("Mcp-Param-Region"));
    }
}
