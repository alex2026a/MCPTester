//! 调用记录仓储（call_records）：持久化、查询、删除、保留清理、大结果落盘
//!
//! 大结果策略（见 03 文档第 5 节）：序列化结果 > 1MB 时截断入库并写
//! `{数据目录}/results/{record_id}.json`；结果目录总量上限 1GB（超限删最旧）。

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::mcp::models::{
    CallMethod, CallOrigin, CallRecord, CallRecordSummary, CallStatus, TransportKind,
};

/// 大结果阈值（1MB）
const TRUNCATE_THRESHOLD: u64 = 1024 * 1024;
/// 落盘目录总量上限（1GB）
const RESULTS_DIR_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// 历史筛选条件
#[derive(Debug, Clone, Default)]
pub struct CallFilter {
    pub server_id: Option<String>,
    pub method: Option<String>,
    pub status: Option<String>,
    /// manual / test；None = 全部
    pub origin: Option<String>,
    pub keyword: Option<String>,
    /// true=同时搜索 params/result 全文（慢查询）
    pub keyword_full: bool,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// 插入结果（供调用方回写 `truncated/size_bytes`）。
#[derive(Debug, Clone, Copy)]
pub struct InsertOutcome {
    pub truncated: bool,
    pub size_bytes: u64,
}

/// 插入记录；大结果截断并落盘。
///
/// 使用普通 `INSERT`：call_id 重复时返回错误，避免静默覆盖旧记录。
pub fn insert(
    conn: &Connection,
    results_dir: &Path,
    record: &CallRecord,
) -> anyhow::Result<InsertOutcome> {
    let result_text = record
        .result
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let size_bytes = result_text
        .as_ref()
        .map(|text| text.len() as u64)
        .unwrap_or(0);

    let (stored_result, truncated) = match result_text {
        Some(text) if size_bytes > TRUNCATE_THRESHOLD => {
            std::fs::create_dir_all(results_dir)?;
            std::fs::write(results_dir.join(format!("{}.json", record.id)), text.as_bytes())?;
            // 截断入库（保留前 1MB，标记 truncated）
            let truncated_text: String = text.chars().take(TRUNCATE_THRESHOLD as usize).collect();
            (Some(truncated_text), true)
        }
        other => (other, false),
    };
    let error_text = record
        .error
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let error_text = error_text.map(|text| {
        if text.len() as u64 > TRUNCATE_THRESHOLD {
            text.chars().take(TRUNCATE_THRESHOLD as usize).collect()
        } else {
            text
        }
    });

    conn.execute(
        "INSERT INTO call_records
            (id, server_id, server_name, method, target, params_json, result_json, error_json,
             status, duration_ms, protocol_version, transport, truncated, size_bytes, origin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            record.id,
            record.server_id,
            record.server_name,
            record.method.as_str(),
            record.target,
            serde_json::to_string(&record.params)?,
            stored_result,
            error_text,
            record.status.as_str(),
            record.duration_ms as i64,
            record.protocol_version,
            record.transport.as_str(),
            if truncated { 1 } else { 0 },
            size_bytes as i64,
            record.origin.as_str(),
            record.created_at.to_rfc3339(),
        ],
    )?;
    Ok(InsertOutcome {
        truncated,
        size_bytes,
    })
}

/// 查询记录列表（分页 + 筛选）
pub fn list(
    conn: &Connection,
    filter: &CallFilter,
    offset: u64,
    limit: u64,
) -> anyhow::Result<(Vec<CallRecordSummary>, u64)> {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(server_id) = &filter.server_id {
        conditions.push("server_id = ?".to_string());
        params.push(Box::new(server_id.clone()));
    }
    if let Some(method) = &filter.method {
        conditions.push("method = ?".to_string());
        params.push(Box::new(method.clone()));
    }
    if let Some(status) = &filter.status {
        conditions.push("status = ?".to_string());
        params.push(Box::new(status.clone()));
    }
    if let Some(origin) = &filter.origin {
        conditions.push("origin = ?".to_string());
        params.push(Box::new(origin.clone()));
    }
    if let Some(from) = &filter.from {
        conditions.push("created_at >= ?".to_string());
        params.push(Box::new(from.to_rfc3339()));
    }
    if let Some(to) = &filter.to {
        conditions.push("created_at <= ?".to_string());
        params.push(Box::new(to.to_rfc3339()));
    }
    if let Some(keyword) = &filter.keyword {
        let keyword = keyword.trim();
        if !keyword.is_empty() {
            if filter.keyword_full {
                conditions.push(
                    "(target LIKE ? OR server_name LIKE ? OR params_json LIKE ? OR result_json LIKE ?)"
                        .to_string(),
                );
                let like = format!("%{keyword}%");
                params.push(Box::new(like.clone()));
                params.push(Box::new(like.clone()));
                params.push(Box::new(like.clone()));
                params.push(Box::new(like));
            } else {
                conditions.push("(target LIKE ? OR server_name LIKE ?)".to_string());
                let like = format!("%{keyword}%");
                params.push(Box::new(like.clone()));
                params.push(Box::new(like));
            }
        }
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM call_records {where_clause}"),
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        |row| row.get(0),
    )?;

    let sql = format!(
        "SELECT id, server_id, server_name, method, target, status, duration_ms, size_bytes,
                truncated, origin, created_at
         FROM call_records {where_clause}
         ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
        params.len() + 1,
        params.len() + 2
    );
    let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = params;
    all_params.push(Box::new(limit as i64));
    all_params.push(Box::new(offset as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(all_params.iter().map(|p| p.as_ref())),
        |row| {
            Ok(SummaryRow {
                id: row.get(0)?,
                server_id: row.get(1)?,
                server_name: row.get(2)?,
                method: row.get(3)?,
                target: row.get(4)?,
                status: row.get(5)?,
                duration_ms: row.get::<_, i64>(6)? as u64,
                size_bytes: row.get::<_, i64>(7)? as u64,
                truncated: row.get::<_, i64>(8)? != 0,
                origin: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row?.into_summary()?);
    }
    Ok((items, total as u64))
}

/// 读取单条完整记录
pub fn get(conn: &Connection, id: &str) -> anyhow::Result<Option<CallRecord>> {
    let row = conn
        .query_row(
            "SELECT id, server_id, server_name, method, target, params_json, result_json,
                    error_json, status, duration_ms, protocol_version, transport, truncated,
                    size_bytes, origin, created_at
             FROM call_records WHERE id = ?1",
            [id],
            |row| {
                Ok(FullRow {
                    id: row.get(0)?,
                    server_id: row.get(1)?,
                    server_name: row.get(2)?,
                    method: row.get(3)?,
                    target: row.get(4)?,
                    params_json: row.get(5)?,
                    result_json: row.get(6)?,
                    error_json: row.get(7)?,
                    status: row.get(8)?,
                    duration_ms: row.get::<_, i64>(9)? as u64,
                    protocol_version: row.get(10)?,
                    transport: row.get(11)?,
                    truncated: row.get::<_, i64>(12)? != 0,
                    size_bytes: row.get::<_, i64>(13)? as u64,
                    origin: row.get(14)?,
                    created_at: row.get(15)?,
                })
            },
        )
        .optional()?;
    match row {
        Some(row) => Ok(Some(row.into_record()?)),
        None => Ok(None),
    }
}

/// 删除指定记录（同时清理落盘文件）
pub fn delete(conn: &Connection, results_dir: &Path, ids: &[String]) -> anyhow::Result<u64> {
    let mut deleted = 0;
    for id in ids {
        deleted += conn.execute("DELETE FROM call_records WHERE id = ?1", [id])? as u64;
        let _ = std::fs::remove_file(results_dir.join(format!("{id}.json")));
    }
    Ok(deleted)
}

/// 清空（按筛选条件；无条件 = 全部）
pub fn clear(conn: &Connection, results_dir: &Path, filter: &CallFilter) -> anyhow::Result<u64> {
    // 先收集待删除 ID（用于清理落盘文件）
    let (items, _) = list(conn, filter, 0, u64::MAX / 2)?;
    let ids: Vec<String> = items.into_iter().map(|item| item.id).collect();
    delete(conn, results_dir, &ids)
}

/// 保留清理：按天数/条数删除旧记录并清理落盘文件（全部来源）。
#[allow(dead_code)]
pub fn cleanup(
    conn: &Connection,
    results_dir: &Path,
    retention_days: u32,
    max_rows: u32,
) -> anyhow::Result<(u64, u64)> {
    cleanup_origin(conn, results_dir, None, retention_days, max_rows)
}

/// 按来源清理（manual / test 分开配额）。
pub fn cleanup_origin(
    conn: &Connection,
    results_dir: &Path,
    origin: Option<&str>,
    retention_days: u32,
    max_rows: u32,
) -> anyhow::Result<(u64, u64)> {
    let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
    let (origin_clause, origin_value) = match origin {
        Some(value) => (" AND origin = ?2", Some(value.to_string())),
        None => ("", None),
    };
    let expired: Vec<String> = {
        let sql = format!("SELECT id FROM call_records WHERE created_at < ?1{origin_clause}");
        let mut stmt = conn.prepare(&sql)?;
        if let Some(origin) = &origin_value {
            stmt.query_map(rusqlite::params![&cutoff, origin], |row| row.get::<_, String>(0))?
                .filter_map(Result::ok)
                .collect()
        } else {
            stmt.query_map([&cutoff], |row| row.get::<_, String>(0))?
                .filter_map(Result::ok)
                .collect()
        }
    };
    let mut deleted = delete(conn, results_dir, &expired)?;

    // 条数上限：删除最旧的超出部分
    let total: i64 = match &origin_value {
        Some(origin) => conn.query_row(
            "SELECT COUNT(*) FROM call_records WHERE origin = ?1",
            [origin],
            |row| row.get(0),
        )?,
        None => conn.query_row("SELECT COUNT(*) FROM call_records", [], |row| row.get(0))?,
    };
    let overflow = total - max_rows as i64;
    if overflow > 0 {
        let overflow_ids: Vec<String> = {
            let sql = match &origin_value {
                Some(_) => "SELECT id FROM call_records WHERE origin = ?1 ORDER BY created_at ASC LIMIT ?2",
                None => "SELECT id FROM call_records ORDER BY created_at ASC LIMIT ?1",
            };
            let mut stmt = conn.prepare(sql)?;
            if let Some(origin) = &origin_value {
                stmt.query_map(rusqlite::params![origin, overflow], |row| row.get::<_, String>(0))?
                    .filter_map(Result::ok)
                    .collect()
            } else {
                stmt.query_map([overflow], |row| row.get::<_, String>(0))?
                    .filter_map(Result::ok)
                    .collect()
            }
        };
        deleted += delete(conn, results_dir, &overflow_ids)?;
    }

    // 落盘目录总量上限：按修改时间删最旧文件
    let mut files_removed = 0u64;
    if let Ok(entries) = std::fs::read_dir(results_dir) {
        let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                Some((entry.path(), meta.len(), meta.modified().ok()?))
            })
            .collect();
        let mut total_bytes: u64 = files.iter().map(|(_, size, _)| *size).sum();
        files.sort_by_key(|(_, _, modified)| *modified);
        let mut index = 0;
        while total_bytes > RESULTS_DIR_MAX_BYTES && index < files.len() {
            let (path, size, _) = &files[index];
            if std::fs::remove_file(path).is_ok() {
                total_bytes = total_bytes.saturating_sub(*size);
                files_removed += 1;
            }
            index += 1;
        }
    }

    Ok((deleted, files_removed))
}

/// 导出（按筛选条件，返回写入的文件路径）
pub fn export(
    conn: &Connection,
    filter: &CallFilter,
    path: &Path,
) -> anyhow::Result<u64> {
    let (json, count) = export_text(conn, filter)?;
    std::fs::write(path, json)?;
    Ok(count)
}

/// 生成导出 JSON 文本（v0.1.1：供导出预览，不落盘）
pub fn export_text(conn: &Connection, filter: &CallFilter) -> anyhow::Result<(String, u64)> {
    let (items, _) = list(conn, filter, 0, u64::MAX / 2)?;
    let mut records = Vec::with_capacity(items.len());
    for item in &items {
        if let Some(record) = get(conn, &item.id)? {
            records.push(record);
        }
    }
    let count = records.len() as u64;
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "exported_by": format!("MCP Tester v{}", env!("CARGO_PKG_VERSION")),
        "exported_at": Utc::now().to_rfc3339(),
        "count": count,
        "records": records,
    }))?;
    Ok((json, count))
}

struct SummaryRow {
    id: String,
    server_id: String,
    server_name: String,
    method: String,
    target: String,
    status: String,
    duration_ms: u64,
    size_bytes: u64,
    truncated: bool,
    origin: String,
    created_at: String,
}

impl SummaryRow {
    fn into_summary(self) -> anyhow::Result<CallRecordSummary> {
        Ok(CallRecordSummary {
            id: self.id,
            server_id: self.server_id,
            server_name: self.server_name,
            method: parse_method(&self.method)?,
            target: self.target,
            status: parse_status(&self.status)?,
            duration_ms: self.duration_ms,
            size_bytes: self.size_bytes,
            truncated: self.truncated,
            origin: parse_origin(&self.origin)?,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

struct FullRow {
    id: String,
    server_id: String,
    server_name: String,
    method: String,
    target: String,
    params_json: String,
    result_json: Option<String>,
    error_json: Option<String>,
    status: String,
    duration_ms: u64,
    protocol_version: String,
    transport: String,
    truncated: bool,
    size_bytes: u64,
    origin: String,
    created_at: String,
}

impl FullRow {
    fn into_record(self) -> anyhow::Result<CallRecord> {
        Ok(CallRecord {
            id: self.id,
            server_id: self.server_id,
            server_name: self.server_name,
            method: parse_method(&self.method)?,
            target: self.target,
            params: serde_json::from_str(&self.params_json).unwrap_or(serde_json::Value::Null),
            result: self
                .result_json
                .and_then(|text| serde_json::from_str(&text).ok()),
            error: self
                .error_json
                .and_then(|text| serde_json::from_str(&text).ok()),
            status: parse_status(&self.status)?,
            duration_ms: self.duration_ms,
            protocol_version: self.protocol_version,
            transport: parse_transport(&self.transport)?,
            truncated: self.truncated,
            size_bytes: self.size_bytes,
            origin: parse_origin(&self.origin)?,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

fn parse_origin(value: &str) -> anyhow::Result<CallOrigin> {
    match value {
        "manual" => Ok(CallOrigin::Manual),
        "test" => Ok(CallOrigin::Test),
        other => anyhow::bail!("未知调用来源：{other}"),
    }
}

fn parse_method(value: &str) -> anyhow::Result<CallMethod> {
    Ok(match value {
        "tools_call" => CallMethod::ToolsCall,
        "resources_read" => CallMethod::ResourcesRead,
        "prompts_get" => CallMethod::PromptsGet,
        "probe" => CallMethod::Probe,
        other => anyhow::bail!("未知 method：{other}"),
    })
}

fn parse_status(value: &str) -> anyhow::Result<CallStatus> {
    Ok(match value {
        "ok" => CallStatus::Ok,
        "error" => CallStatus::Error,
        "canceled" => CallStatus::Canceled,
        other => anyhow::bail!("未知 status：{other}"),
    })
}

fn parse_transport(value: &str) -> anyhow::Result<TransportKind> {
    Ok(match value {
        "stdio" => TransportKind::Stdio,
        "streamable_http" => TransportKind::StreamableHttp,
        "sse" => TransportKind::Sse,
        other => anyhow::bail!("未知 transport：{other}"),
    })
}

fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::error::CallError;

    fn temp_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let mut conn = Connection::open(dir.path().join("test.db")).expect("数据库打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");
        (dir, conn)
    }

    fn sample(id: &str, method: CallMethod, status: CallStatus) -> CallRecord {
        CallRecord {
            id: id.to_string(),
            server_id: "s1".to_string(),
            server_name: "mock".to_string(),
            method,
            target: "echo".to_string(),
            params: serde_json::json!({ "name": "echo", "arguments": { "text": "hi" } }),
            result: if status == CallStatus::Ok {
                Some(serde_json::json!({ "content": [{ "type": "text", "text": "echo: hi" }] }))
            } else {
                None
            },
            error: if status == CallStatus::Error {
                Some(CallError::Timeout { timeout_secs: 1 })
            } else {
                None
            },
            status,
            duration_ms: 12,
            protocol_version: "2026-07-28".to_string(),
            transport: TransportKind::Stdio,
            truncated: false,
            size_bytes: 0,
            origin: CallOrigin::Manual,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn insert_list_get_delete_roundtrip() {
        let (dir, conn) = temp_conn();
        let results = dir.path().join("results");
        insert(&conn, &results, &sample("c1", CallMethod::ToolsCall, CallStatus::Ok))
            .expect("插入失败");
        insert(&conn, &results, &sample("c2", CallMethod::Probe, CallStatus::Error))
            .expect("插入失败");

        let (items, total) = list(&conn, &CallFilter::default(), 0, 10).expect("查询失败");
        assert_eq!(total, 2);
        assert_eq!(items.len(), 2);

        // 筛选：状态
        let filter = CallFilter {
            status: Some("error".to_string()),
            ..Default::default()
        };
        let (items, total) = list(&conn, &filter, 0, 10).expect("查询失败");
        assert_eq!(total, 1);
        assert_eq!(items[0].id, "c2");

        // 关键词（target/server_name 范围）
        let filter = CallFilter {
            keyword: Some("ech".to_string()),
            ..Default::default()
        };
        let (items, _) = list(&conn, &filter, 0, 10).expect("查询失败");
        assert_eq!(items.len(), 2);

        // 筛选：来源
        let filter = CallFilter {
            origin: Some("manual".to_string()),
            ..Default::default()
        };
        let (_, total) = list(&conn, &filter, 0, 10).expect("查询失败");
        assert_eq!(total, 2);

        let record = get(&conn, "c1").expect("查询失败").expect("应存在");
        assert_eq!(record.method, CallMethod::ToolsCall);
        assert_eq!(record.origin, CallOrigin::Manual);
        assert!(record.result.is_some());

        let deleted = delete(&conn, &results, &["c1".to_string()]).expect("删除失败");
        assert_eq!(deleted, 1);
        assert!(get(&conn, "c1").expect("查询失败").is_none());
    }

    #[test]
    fn large_result_truncates_and_spills_to_file() {
        let (dir, conn) = temp_conn();
        let results = dir.path().join("results");
        let mut record = sample("big", CallMethod::ToolsCall, CallStatus::Ok);
        record.result = Some(serde_json::json!({ "blob": "x".repeat(1_200_000) }));
        insert(&conn, &results, &record).expect("插入失败");

        let stored = get(&conn, "big").expect("查询失败").expect("应存在");
        assert!(stored.truncated, "应标记截断");
        assert!(stored.size_bytes > 1_000_000);
        assert!(
            results.join("big.json").exists(),
            "完整内容应落盘"
        );

        // 删除记录同时删除落盘文件
        delete(&conn, &results, &["big".to_string()]).expect("删除失败");
        assert!(!results.join("big.json").exists());
    }

    #[test]
    fn retention_cleanup_removes_expired() {
        let (dir, conn) = temp_conn();
        let results = dir.path().join("results");
        let mut old = sample("old", CallMethod::ToolsCall, CallStatus::Ok);
        old.created_at = Utc::now() - chrono::Duration::days(60);
        insert(&conn, &results, &old).expect("插入失败");
        insert(&conn, &results, &sample("new", CallMethod::ToolsCall, CallStatus::Ok))
            .expect("插入失败");

        let (deleted, _) = cleanup(&conn, &results, 30, 1000).expect("清理失败");
        assert_eq!(deleted, 1);
        assert!(get(&conn, "old").expect("查询失败").is_none());
        assert!(get(&conn, "new").expect("查询失败").is_some());
    }

    #[test]
    fn export_writes_json() {
        let (dir, conn) = temp_conn();
        let results = dir.path().join("results");
        insert(&conn, &results, &sample("c1", CallMethod::ToolsCall, CallStatus::Ok))
            .expect("插入失败");
        let path = dir.path().join("export.json");
        let count = export(&conn, &CallFilter::default(), &path).expect("导出失败");
        assert_eq!(count, 1);
        let text = std::fs::read_to_string(&path).expect("读取失败");
        assert!(text.contains("echo: hi"));
    }
}
