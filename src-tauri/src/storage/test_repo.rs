//! 测试套件/用例/运行仓储（v0.2.1）。

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::testing::models::{
    TestCase, TestCaseDraft, TestPriority, TestRisk, TestRunStatus, TestSource, TestSuite,
    TestSuiteDraft, TestSuiteSummary, TestSuiteUpdateDraft, TestMethod,
};

/// 列表：按服务器过滤（None=全部），带用例计数与最近运行。
pub fn list_suites(
    conn: &Connection,
    server_id: Option<&str>,
) -> anyhow::Result<Vec<TestSuiteSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.server_id, sv.name,
                (SELECT COUNT(*) FROM test_cases c WHERE c.suite_id = s.id),
                (SELECT COUNT(*) FROM test_cases c WHERE c.suite_id = s.id AND c.enabled = 1),
                s.baseline_run_id,
                (SELECT r.started_at FROM test_runs r WHERE r.suite_id = s.id
                 ORDER BY r.started_at DESC LIMIT 1),
                (SELECT r.status FROM test_runs r WHERE r.suite_id = s.id
                 ORDER BY r.started_at DESC LIMIT 1)
         FROM test_suites s
         LEFT JOIN servers sv ON sv.id = s.server_id
         WHERE (?1 IS NULL OR s.server_id = ?1)
         ORDER BY s.updated_at DESC, s.id",
    )?;
    let rows = stmt.query_map([server_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, i64>(4)? as u64,
            row.get::<_, i64>(5)? as u64,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
        ))
    })?;

    let mut items = Vec::new();
    for row in rows {
        let (
            id,
            name,
            server_id,
            server_name,
            case_count,
            enabled_count,
            baseline_run_id,
            last_run_at,
            last_run_status,
        ) = row?;
        items.push(TestSuiteSummary {
            id,
            name,
            server_id,
            server_name,
            case_count,
            enabled_count,
            baseline_run_id,
            last_run_at: last_run_at.map(|value| parse_time(&value)).transpose()?,
            last_run_status: last_run_status
                .map(|value| parse_enum::<TestRunStatus>(&value))
                .transpose()?,
        });
    }
    Ok(items)
}

pub fn get_suite(conn: &Connection, id: &str) -> anyhow::Result<Option<TestSuite>> {
    let row = conn
        .query_row(
            "SELECT id, name, description, server_id, tags_json, capability_hash,
                    capability_snapshot_json, baseline_run_id, created_at, updated_at
             FROM test_suites WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        name,
        description,
        server_id,
        tags_json,
        capability_hash,
        capability_snapshot_json,
        baseline_run_id,
        created_at,
        updated_at,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(TestSuite {
        id,
        name,
        description,
        server_id,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        capability_hash,
        capability_snapshot: capability_snapshot_json
            .and_then(|text| serde_json::from_str(&text).ok()),
        baseline_run_id,
        created_at: parse_time(&created_at)?,
        updated_at: parse_time(&updated_at)?,
    }))
}

pub fn create_suite(conn: &Connection, draft: &TestSuiteDraft) -> anyhow::Result<TestSuite> {
    let now = Utc::now();
    let suite = TestSuite {
        id: Uuid::new_v4().to_string(),
        name: draft.name.trim().to_string(),
        description: draft.description.trim().to_string(),
        server_id: draft.server_id.clone(),
        tags: draft.tags.clone(),
        capability_hash: None,
        capability_snapshot: None,
        baseline_run_id: None,
        created_at: now,
        updated_at: now,
    };
    conn.execute(
        "INSERT INTO test_suites
            (id, name, description, server_id, tags_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            suite.id,
            suite.name,
            suite.description,
            suite.server_id,
            serde_json::to_string(&suite.tags)?,
            suite.created_at.to_rfc3339(),
            suite.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(suite)
}

pub fn update_suite(
    conn: &Connection,
    id: &str,
    draft: &TestSuiteUpdateDraft,
) -> anyhow::Result<TestSuite> {
    let now = Utc::now();
    let changed = conn.execute(
        "UPDATE test_suites SET name = ?1, description = ?2, tags_json = ?3, updated_at = ?4
         WHERE id = ?5",
        rusqlite::params![
            draft.name.trim(),
            draft.description.trim(),
            serde_json::to_string(&draft.tags)?,
            now.to_rfc3339(),
            id,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("套件不存在");
    }
    get_suite(conn, id)?.ok_or_else(|| anyhow::anyhow!("套件不存在"))
}

/// 仅当套件没有用例时允许重新绑定服务器。
pub fn rebind_suite(conn: &Connection, id: &str, server_id: &str) -> anyhow::Result<TestSuite> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM test_cases WHERE suite_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    if count > 0 {
        anyhow::bail!("套件已有用例，不能更换服务器；请新建套件或导出/导入");
    }
    let changed = conn.execute(
        "UPDATE test_suites SET server_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![server_id, Utc::now().to_rfc3339(), id],
    )?;
    if changed == 0 {
        anyhow::bail!("套件不存在");
    }
    get_suite(conn, id)?.ok_or_else(|| anyhow::anyhow!("套件不存在"))
}

pub fn delete_suite(conn: &Connection, id: &str) -> anyhow::Result<u64> {
    Ok(conn.execute("DELETE FROM test_suites WHERE id = ?1", [id])? as u64)
}

/// 列出套件内用例（顺序稳定：order_index, id）。
pub fn list_cases(conn: &Connection, suite_id: &str) -> anyhow::Result<Vec<TestCase>> {
    let mut stmt = conn.prepare(
        "SELECT id, suite_id, name, description, method, target, arguments_json, expect_json,
                assertions_json, tags_json, priority, risk, source, generation_note, enabled,
                order_index, timeout_secs, retry_count, created_at, updated_at
         FROM test_cases WHERE suite_id = ?1 ORDER BY order_index, id",
    )?;
    let rows = stmt.query_map([suite_id], case_from_row)?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn get_case(conn: &Connection, id: &str) -> anyhow::Result<Option<TestCase>> {
    let row = conn
        .query_row(
            "SELECT id, suite_id, name, description, method, target, arguments_json, expect_json,
                    assertions_json, tags_json, priority, risk, source, generation_note, enabled,
                    order_index, timeout_secs, retry_count, created_at, updated_at
             FROM test_cases WHERE id = ?1",
            [id],
            case_from_row,
        )
        .optional()?;
    Ok(row)
}

/// 批量插入（分配 MAX(order_index)+1），在调用方事务中执行。
pub fn insert_cases(
    conn: &Connection,
    suite_id: &str,
    drafts: &[TestCaseDraft],
) -> anyhow::Result<Vec<TestCase>> {
    let mut next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(order_index), -1) + 1 FROM test_cases WHERE suite_id = ?1",
        [suite_id],
        |row| row.get(0),
    )?;
    let now = Utc::now();
    let mut inserted = Vec::new();
    for draft in drafts {
        let case = TestCase {
            id: Uuid::new_v4().to_string(),
            suite_id: suite_id.to_string(),
            name: draft.name.trim().to_string(),
            description: draft.description.trim().to_string(),
            method: draft.method.clone(),
            target: draft.target.trim().to_string(),
            arguments: draft.arguments.clone(),
            expect: draft.expect.clone(),
            assertions: draft.assertions.clone(),
            tags: draft.tags.clone(),
            priority: draft.priority.clone(),
            risk: draft.risk.clone(),
            source: draft.source.clone(),
            generation_note: draft.generation_note.clone(),
            enabled: draft.enabled,
            order_index: next,
            timeout_secs: draft.timeout_secs,
            retry_count: draft.retry_count,
            created_at: now,
            updated_at: now,
        };
        insert_case_row(conn, &case)?;
        inserted.push(case);
        next += 1;
    }
    touch_suite(conn, suite_id)?;
    Ok(inserted)
}

pub fn update_case(conn: &Connection, id: &str, draft: &TestCaseDraft) -> anyhow::Result<TestCase> {
    let now = Utc::now();
    let changed = conn.execute(
        "UPDATE test_cases SET name = ?1, description = ?2, method = ?3, target = ?4,
                arguments_json = ?5, expect_json = ?6, assertions_json = ?7, tags_json = ?8,
                priority = ?9, risk = ?10, source = ?11, generation_note = ?12, enabled = ?13,
                timeout_secs = ?14, retry_count = ?15, updated_at = ?16
         WHERE id = ?17",
        rusqlite::params![
            draft.name.trim(),
            draft.description.trim(),
            draft.method.as_str(),
            draft.target.trim(),
            serde_json::to_string(&draft.arguments)?,
            serde_json::to_string(&draft.expect)?,
            serde_json::to_string(&draft.assertions)?,
            serde_json::to_string(&draft.tags)?,
            priority_str(&draft.priority),
            risk_str(&draft.risk),
            source_str(&draft.source),
            draft.generation_note,
            if draft.enabled { 1 } else { 0 },
            draft.timeout_secs.map(|value| value as i64),
            draft.retry_count as i64,
            now.to_rfc3339(),
            id,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("用例不存在");
    }
    let case = get_case(conn, id)?.ok_or_else(|| anyhow::anyhow!("用例不存在"))?;
    touch_suite(conn, &case.suite_id)?;
    Ok(case)
}

pub fn delete_cases(conn: &Connection, ids: &[String]) -> anyhow::Result<u64> {
    let mut deleted = 0u64;
    for id in ids {
        if let Some(case) = get_case(conn, id)? {
            deleted += conn.execute("DELETE FROM test_cases WHERE id = ?1", [id])? as u64;
            touch_suite(conn, &case.suite_id)?;
        }
    }
    Ok(deleted)
}

pub fn duplicate_case(conn: &Connection, id: &str) -> anyhow::Result<TestCase> {
    let source = get_case(conn, id)?.ok_or_else(|| anyhow::anyhow!("用例不存在"))?;
    let draft = TestCaseDraft {
        name: format!("{}（副本）", source.name),
        description: source.description,
        method: source.method,
        target: source.target,
        arguments: source.arguments,
        expect: source.expect,
        assertions: source.assertions,
        tags: source.tags,
        priority: source.priority,
        risk: source.risk,
        source: TestSource::Manual,
        generation_note: source.generation_note,
        enabled: source.enabled,
        timeout_secs: source.timeout_secs,
        retry_count: source.retry_count,
    };
    let mut inserted = insert_cases(conn, &source.suite_id, &[draft])?;
    inserted.pop().ok_or_else(|| anyhow::anyhow!("复制失败"))
}

/// 按传入 id 顺序重排；ids 必须全部属于同一套件。
pub fn reorder_cases(conn: &Connection, suite_id: &str, ids: &[String]) -> anyhow::Result<()> {
    let existing: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM test_cases WHERE suite_id = ?1 ORDER BY order_index, id")?;
        let rows = stmt.query_map([suite_id], |row| row.get::<_, String>(0))?;
        rows.filter_map(Result::ok).collect()
    };
    if existing.len() != ids.len()
        || existing.iter().any(|id| !ids.contains(id))
        || ids.iter().any(|id| !existing.contains(id))
    {
        anyhow::bail!("排序列表必须完整覆盖该套件的用例");
    }
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE test_cases SET order_index = ?1 WHERE id = ?2 AND suite_id = ?3",
            rusqlite::params![index as i64, id, suite_id],
        )?;
    }
    touch_suite(conn, suite_id)?;
    Ok(())
}

/// 套件指纹/能力快照写入（`save_generated_cases` 与手动保存共用；M2 使用）。
#[allow(dead_code)]
pub fn set_suite_capability(
    conn: &Connection,
    suite_id: &str,
    hash: &str,
    snapshot: &Value,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE test_suites SET capability_hash = ?1, capability_snapshot_json = ?2,
                updated_at = ?3 WHERE id = ?4",
        rusqlite::params![
            hash,
            serde_json::to_string(snapshot)?,
            Utc::now().to_rfc3339(),
            suite_id
        ],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn set_baseline(conn: &Connection, run_id: &str) -> anyhow::Result<()> {
    let (suite_id, status): (Option<String>, String) = conn
        .query_row(
            "SELECT suite_id, status FROM test_runs WHERE id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("运行不存在"))?;
    let suite_id = suite_id.ok_or_else(|| anyhow::anyhow!("该运行不属于任何套件，不能设为基线"))?;
    if status != "completed" && status != "failed" {
        anyhow::bail!("只有已完成/失败的运行可以设为基线");
    }
    conn.execute(
        "UPDATE test_suites SET baseline_run_id = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![run_id, Utc::now().to_rfc3339(), suite_id],
    )?;
    Ok(())
}

/// 清理过期/超量的运行记录；删除前清空套件基线引用。
pub fn cleanup_runs(
    conn: &Connection,
    reports_dir: &std::path::Path,
    retention_days: u32,
    max_rows: u32,
) -> anyhow::Result<u64> {
    let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
    let mut ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM test_runs WHERE started_at < ?1")?;
        stmt.query_map([&cutoff], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect()
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM test_runs", [], |row| row.get(0))?;
    let overflow = total - max_rows as i64;
    if overflow > 0 {
        let mut stmt =
            conn.prepare("SELECT id FROM test_runs ORDER BY started_at ASC LIMIT ?1")?;
        let old: Vec<String> = stmt
            .query_map([overflow], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        for id in old {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }
    for id in &ids {
        conn.execute(
            "UPDATE test_suites SET baseline_run_id = NULL WHERE baseline_run_id = ?1",
            [id],
        )?;
    }
    let mut deleted = 0u64;
    for id in ids {
        deleted += conn.execute("DELETE FROM test_runs WHERE id = ?1", [&id])? as u64;
        for extension in ["html", "json", "xml", "md"] {
            let _ = std::fs::remove_file(reports_dir.join(format!("{id}.{extension}")));
        }
    }
    Ok(deleted)
}

/// 启动时回收上次异常退出留下的 running 运行。
pub fn recover_interrupted_runs(conn: &Connection) -> anyhow::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let changed = conn.execute(
        "UPDATE test_runs SET status = 'interrupted', stop_reason = 'interrupted',
                finished_at = ?1 WHERE status = 'running'",
        [now],
    )?;
    Ok(changed as u64)
}

fn insert_case_row(conn: &Connection, case: &TestCase) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO test_cases
            (id, suite_id, name, description, method, target, arguments_json, expect_json,
             assertions_json, tags_json, priority, risk, source, generation_note, enabled,
             order_index, timeout_secs, retry_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                 ?17, ?18, ?19, ?20)",
        rusqlite::params![
            case.id,
            case.suite_id,
            case.name,
            case.description,
            case.method.as_str(),
            case.target,
            serde_json::to_string(&case.arguments)?,
            serde_json::to_string(&case.expect)?,
            serde_json::to_string(&case.assertions)?,
            serde_json::to_string(&case.tags)?,
            priority_str(&case.priority),
            risk_str(&case.risk),
            source_str(&case.source),
            case.generation_note,
            if case.enabled { 1 } else { 0 },
            case.order_index,
            case.timeout_secs.map(|value| value as i64),
            case.retry_count as i64,
            case.created_at.to_rfc3339(),
            case.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn case_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestCase> {
    Ok(TestCase {
        id: row.get(0)?,
        suite_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        method: parse_enum(&row.get::<_, String>(4)?).map_err(sql_err)?,
        target: row.get(5)?,
        arguments: parse_json(row.get::<_, String>(6)?),
        expect: parse_json_required(row.get::<_, String>(7)?).map_err(sql_err)?,
        assertions: parse_json_default(row.get::<_, String>(8)?),
        tags: parse_json_default(row.get::<_, String>(9)?),
        priority: parse_enum(&row.get::<_, String>(10)?).map_err(sql_err)?,
        risk: parse_enum(&row.get::<_, String>(11)?).map_err(sql_err)?,
        source: parse_enum(&row.get::<_, String>(12)?).map_err(sql_err)?,
        generation_note: row.get(13)?,
        enabled: row.get::<_, i64>(14)? != 0,
        order_index: row.get(15)?,
        timeout_secs: row.get::<_, Option<i64>>(16)?.map(|value| value as u64),
        retry_count: row.get::<_, i64>(17)? as u32,
        created_at: parse_time(&row.get::<_, String>(18)?).map_err(sql_err)?,
        updated_at: parse_time(&row.get::<_, String>(19)?).map_err(sql_err)?,
    })
}

fn sql_err(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn touch_suite(conn: &Connection, suite_id: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE test_suites SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![Utc::now().to_rfc3339(), suite_id],
    )?;
    Ok(())
}

fn priority_str(value: &TestPriority) -> &'static str {
    match value {
        TestPriority::P0 => "p0",
        TestPriority::P1 => "p1",
        TestPriority::P2 => "p2",
    }
}

fn risk_str(value: &TestRisk) -> &'static str {
    match value {
        TestRisk::ReadOnly => "read_only",
        TestRisk::Write => "write",
        TestRisk::Unknown => "unknown",
        TestRisk::Destructive => "destructive",
    }
}

fn source_str(value: &TestSource) -> &'static str {
    match value {
        TestSource::Manual => "manual",
        TestSource::Schema => "schema",
        TestSource::History => "history",
        TestSource::Llm => "llm",
        TestSource::Import => "import",
    }
}

fn parse_enum<T: DeserializeOwned>(value: &str) -> anyhow::Result<T> {
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|error| anyhow::anyhow!("非法枚举值 {value}：{error}"))
}

fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn parse_json(value: String) -> Value {
    serde_json::from_str(&value).unwrap_or_else(|_| json!({}))
}

fn parse_json_required<T: DeserializeOwned>(value: String) -> anyhow::Result<T> {
    Ok(serde_json::from_str(&value)?)
}

fn parse_json_default<T: DeserializeOwned + Default>(value: String) -> T {
    serde_json::from_str(&value).unwrap_or_default()
}

/// 运行用例写入行（M3 运行器使用）。
#[derive(Clone)]
pub struct RunCaseRow {
    pub id: String,
    pub run_id: String,
    pub case_id: Option<String>,
    pub case_name: String,
    pub method: TestMethod,
    pub target: String,
    pub order_index: i64,
    pub case_snapshot: Value,
    pub status: crate::testing::models::TestRunCaseStatus,
    pub failure_kind: Option<crate::testing::models::FailureKind>,
    pub skip_reason: Option<String>,
    pub duration_ms: u64,
    pub call_record_id: Option<String>,
    pub attempt_record_ids: Vec<String>,
    pub attempts: u32,
    pub assertions: Vec<crate::testing::models::AssertionResult>,
    pub evidence: crate::testing::models::Evidence,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn create_run(conn: &Connection, run: &crate::testing::models::TestRun) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO test_runs
            (id, suite_id, suite_name, server_id, server_name, status, stop_reason,
             options_json, manifest_json, summary_json, baseline_run_id, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            run.id,
            run.suite_id,
            run.suite_name,
            run.server_id,
            run.server_name,
            run.status.as_str(),
            run.stop_reason,
            serde_json::to_string(&run.options)?,
            serde_json::to_string(&run.manifest)?,
            serde_json::to_string(&run.summary)?,
            run.baseline_run_id,
            run.started_at.to_rfc3339(),
            run.finished_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    Ok(())
}

pub fn update_run_summary(
    conn: &Connection,
    run_id: &str,
    summary: &crate::testing::models::RunSummary,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE test_runs SET summary_json = ?1 WHERE id = ?2",
        rusqlite::params![serde_json::to_string(summary)?, run_id],
    )?;
    Ok(())
}

pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    status: crate::testing::models::TestRunStatus,
    stop_reason: Option<&str>,
    summary: &crate::testing::models::RunSummary,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE test_runs SET status = ?1, stop_reason = ?2, summary_json = ?3,
                finished_at = ?4 WHERE id = ?5",
        rusqlite::params![
            status.as_str(),
            stop_reason,
            serde_json::to_string(summary)?,
            Utc::now().to_rfc3339(),
            run_id,
        ],
    )?;
    Ok(())
}

pub fn insert_run_case(conn: &Connection, row: &RunCaseRow) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO test_run_cases
            (id, run_id, case_id, case_name, method, target, order_index, case_snapshot_json,
             status, failure_kind, skip_reason, duration_ms, call_record_id,
             attempt_record_ids_json, attempts, assertions_json, evidence_json, message, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        rusqlite::params![
            row.id,
            row.run_id,
            row.case_id,
            row.case_name,
            row.method.as_str(),
            row.target,
            row.order_index,
            serde_json::to_string(&row.case_snapshot)?,
            row.status.as_str(),
            row.failure_kind.as_ref().map(|kind| kind.as_str()),
            row.skip_reason,
            row.duration_ms as i64,
            row.call_record_id,
            serde_json::to_string(&row.attempt_record_ids)?,
            row.attempts as i64,
            serde_json::to_string(&row.assertions)?,
            serde_json::to_string(&row.evidence)?,
            row.message,
            row.created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_run(
    conn: &Connection,
    run_id: &str,
) -> anyhow::Result<Option<crate::testing::models::TestRun>> {
    let row = conn
        .query_row(
            "SELECT id, suite_id, suite_name, server_id, server_name, status, stop_reason,
                    options_json, manifest_json, summary_json, baseline_run_id, started_at, finished_at
             FROM test_runs WHERE id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        suite_id,
        suite_name,
        server_id,
        server_name,
        status,
        stop_reason,
        options_json,
        manifest_json,
        summary_json,
        baseline_run_id,
        started_at,
        finished_at,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(crate::testing::models::TestRun {
        id,
        suite_id,
        suite_name,
        server_id,
        server_name,
        status: parse_enum(&status)?,
        stop_reason,
        options: serde_json::from_str(&options_json).unwrap_or_default(),
        manifest: serde_json::from_str(&manifest_json)?,
        summary: serde_json::from_str(&summary_json).unwrap_or_default(),
        baseline_run_id,
        started_at: parse_time(&started_at)?,
        finished_at: finished_at.map(|value| parse_time(&value)).transpose()?,
    }))
}

/// 运行列表（筛选 + 分页）。
pub fn list_runs(
    conn: &Connection,
    filter: &crate::testing::models::TestRunFilter,
    offset: u64,
    limit: u64,
) -> anyhow::Result<(Vec<crate::testing::models::TestRunSummary>, u64)> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(server_id) = &filter.server_id {
        conditions.push("r.server_id = ?".to_string());
        params.push(Box::new(server_id.clone()));
    }
    if let Some(suite_id) = &filter.suite_id {
        conditions.push("r.suite_id = ?".to_string());
        params.push(Box::new(suite_id.clone()));
    }
    if let Some(status) = &filter.status {
        conditions.push("r.status = ?".to_string());
        params.push(Box::new(status.as_str().to_string()));
    }
    if let Some(from) = &filter.from {
        conditions.push("r.started_at >= ?".to_string());
        params.push(Box::new(from.to_rfc3339()));
    }
    if let Some(to) = &filter.to {
        conditions.push("r.started_at <= ?".to_string());
        params.push(Box::new(to.to_rfc3339()));
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM test_runs r {where_clause}"),
        rusqlite::params_from_iter(params.iter().map(|item| item.as_ref())),
        |row| row.get(0),
    )?;
    let sql = format!(
        "SELECT r.id, r.suite_id, r.suite_name, r.server_id, r.server_name, r.status,
                r.summary_json, r.started_at, r.finished_at,
                EXISTS(SELECT 1 FROM test_suites s WHERE s.id = r.suite_id AND s.baseline_run_id = r.id)
         FROM test_runs r {where_clause}
         ORDER BY r.started_at DESC LIMIT ?{} OFFSET ?{}",
        params.len() + 1,
        params.len() + 2
    );
    let mut all_params = params;
    all_params.push(Box::new(limit as i64));
    all_params.push(Box::new(offset as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(all_params.iter().map(|item| item.as_ref())),
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)? != 0,
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (
            id,
            suite_id,
            suite_name,
            server_id,
            server_name,
            status,
            summary_json,
            started_at,
            finished_at,
            baseline,
        ) = row?;
        let summary: crate::testing::models::RunSummary =
            serde_json::from_str(&summary_json).unwrap_or_default();
        items.push(crate::testing::models::TestRunSummary {
            id,
            suite_id,
            suite_name,
            server_id,
            server_name,
            status: parse_enum(&status)?,
            total: summary.total,
            passed: summary.passed,
            failed: summary.failed,
            warning: summary.warning,
            skipped: summary.skipped,
            error: summary.error,
            canceled: summary.canceled,
            pass_rate: summary.pass_rate,
            effective_pass_rate: summary.effective_pass_rate,
            duration_ms: summary.duration_ms,
            started_at: parse_time(&started_at)?,
            finished_at: finished_at.map(|value| parse_time(&value)).transpose()?,
            baseline,
        });
    }
    Ok((items, total as u64))
}

/// 运行用例列表（筛选 + 分页）。
pub fn list_run_cases(
    conn: &Connection,
    run_id: &str,
    filter: &crate::testing::models::TestRunCaseFilter,
    offset: u64,
    limit: u64,
) -> anyhow::Result<(Vec<crate::testing::models::TestRunCaseSummary>, u64)> {
    let mut conditions = vec!["run_id = ?".to_string()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(run_id.to_string())];
    if let Some(status) = &filter.status {
        conditions.push("status = ?".to_string());
        params.push(Box::new(status.as_str().to_string()));
    }
    if let Some(kind) = &filter.failure_kind {
        conditions.push("failure_kind = ?".to_string());
        params.push(Box::new(kind.as_str().to_string()));
    }
    if let Some(keyword) = &filter.keyword {
        conditions.push("(case_name LIKE ? OR target LIKE ?)".to_string());
        let like = format!("%{}%", keyword.trim());
        params.push(Box::new(like.clone()));
        params.push(Box::new(like));
    }
    let where_clause = format!("WHERE {}", conditions.join(" AND "));
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM test_run_cases {where_clause}"),
        rusqlite::params_from_iter(params.iter().map(|item| item.as_ref())),
        |row| row.get(0),
    )?;
    let sql = format!(
        "SELECT id, case_id, case_name, method, target, order_index, status, failure_kind,
                skip_reason, duration_ms, attempts, call_record_id, message
         FROM test_run_cases {where_clause}
         ORDER BY order_index, id LIMIT ?{} OFFSET ?{}",
        params.len() + 1,
        params.len() + 2
    );
    let mut all_params = params;
    all_params.push(Box::new(limit as i64));
    all_params.push(Box::new(offset as i64));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(all_params.iter().map(|item| item.as_ref())),
        run_case_summary_from_row,
    )?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok((items, total as u64))
}

pub fn get_run_case(
    conn: &Connection,
    id: &str,
) -> anyhow::Result<Option<crate::testing::models::TestRunCaseDetail>> {
    let row = conn
        .query_row(
            "SELECT id, case_id, case_name, method, target, order_index, status, failure_kind,
                    skip_reason, duration_ms, attempts, call_record_id, message,
                    case_snapshot_json, attempt_record_ids_json, assertions_json, evidence_json
             FROM test_run_cases WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    run_case_summary_from_row(row)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                ))
            },
        )
        .optional()?;
    let Some((summary, snapshot, attempts, assertions, evidence)) = row else {
        return Ok(None);
    };
    Ok(Some(crate::testing::models::TestRunCaseDetail {
        summary,
        case_snapshot: serde_json::from_str(&snapshot).unwrap_or(Value::Null),
        attempt_record_ids: serde_json::from_str(&attempts).unwrap_or_default(),
        assertions: serde_json::from_str(&assertions).unwrap_or_default(),
        evidence: serde_json::from_str(&evidence).unwrap_or_default(),
    }))
}

pub fn get_all_run_cases(
    conn: &Connection,
    run_id: &str,
) -> anyhow::Result<Vec<crate::testing::models::TestRunCaseDetail>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM test_run_cases WHERE run_id = ?1 ORDER BY order_index, id",
    )?;
    let ids: Vec<String> = stmt
        .query_map([run_id], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect();
    let mut cases = Vec::new();
    for id in ids {
        if let Some(case) = get_run_case(conn, &id)? {
            cases.push(case);
        }
    }
    Ok(cases)
}

fn run_case_summary_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::testing::models::TestRunCaseSummary> {
    let failure_kind: Option<String> = row.get(7)?;
    Ok(crate::testing::models::TestRunCaseSummary {
        id: row.get(0)?,
        case_id: row.get(1)?,
        case_name: row.get(2)?,
        method: parse_enum(&row.get::<_, String>(3)?).map_err(sql_err)?,
        target: row.get(4)?,
        order_index: row.get(5)?,
        status: parse_enum(&row.get::<_, String>(6)?).map_err(sql_err)?,
        failure_kind: failure_kind
            .map(|value| parse_enum(&value))
            .transpose()
            .map_err(sql_err)?,
        skip_reason: row.get(8)?,
        duration_ms: row.get::<_, i64>(9)? as u64,
        attempts: row.get::<_, i64>(10)? as u32,
        call_record_id: row.get(11)?,
        message: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::models::{TestExpect, TestMethod, TestOutcome};

    fn conn() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut conn = Connection::open(dir.path().join("test.db")).expect("打开");
        conn.pragma_update(None, "foreign_keys", "ON").expect("fk");
        crate::storage::migrations::run(&mut conn).expect("迁移");
        (dir, conn)
    }

    fn draft(name: &str) -> TestCaseDraft {
        TestCaseDraft {
            name: name.to_string(),
            method: TestMethod::ToolsCall,
            target: "echo".to_string(),
            arguments: json!({"text": "hi"}),
            expect: TestExpect {
                outcome: TestOutcome::Success,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn suite(conn: &Connection) -> TestSuite {
        create_suite(
            conn,
            &TestSuiteDraft {
                name: "套件".to_string(),
                description: String::new(),
                server_id: "s1".to_string(),
                tags: Vec::new(),
            },
        )
        .expect("创建套件")
    }

    #[test]
    fn suite_and_case_crud() {
        let (_dir, conn) = conn();
        let suite = suite(&conn);
        let cases = insert_cases(&conn, &suite.id, &[draft("a"), draft("b")]).expect("插入");
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].order_index, 0);
        assert_eq!(cases[1].order_index, 1);

        let mut updated = draft("a2");
        updated.enabled = false;
        let case = update_case(&conn, &cases[0].id, &updated).expect("更新");
        assert_eq!(case.name, "a2");
        assert!(!case.enabled);

        let copy = duplicate_case(&conn, &cases[1].id).expect("复制");
        assert!(copy.name.contains("副本"));

        reorder_cases(&conn, &suite.id, &[copy.id.clone(), cases[0].id.clone(), cases[1].id.clone()])
            .expect("排序");
        let listed = list_cases(&conn, &suite.id).expect("列表");
        assert_eq!(listed[0].id, copy.id);

        assert_eq!(delete_cases(&conn, &[copy.id]).expect("删除"), 1);
        assert_eq!(list_cases(&conn, &suite.id).expect("列表").len(), 2);
    }

    #[test]
    fn cascade_delete_cases_and_keep_runs() {
        let (_dir, conn) = conn();
        let suite = suite(&conn);
        insert_cases(&conn, &suite.id, &[draft("a")]).expect("插入");
        conn.execute(
            "INSERT INTO test_runs
                (id, suite_id, suite_name, server_id, server_name, status, options_json,
                 manifest_json, summary_json, started_at)
             VALUES ('r1', ?1, '套件', 's1', 'mock', 'completed', '{}', '{}', '{}', '2026-01-01T00:00:00Z')",
            [&suite.id],
        )
        .expect("运行");
        delete_suite(&conn, &suite.id).expect("删除套件");
        assert_eq!(list_cases(&conn, &suite.id).expect("列表").len(), 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test_runs WHERE id = 'r1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn rebind_only_when_empty() {
        let (_dir, conn) = conn();
        let suite = suite(&conn);
        rebind_suite(&conn, &suite.id, "s2").expect("空套件可重绑");
        insert_cases(&conn, &suite.id, &[draft("a")]).expect("插入");
        assert!(rebind_suite(&conn, &suite.id, "s3").is_err());
    }

    #[test]
    fn run_case_roundtrip_and_finish() {
        use crate::testing::models::{
            Evidence, RunManifest, RunOptions, RunSummary, TestRun, TestRunCaseStatus,
            TestRunStatus,
        };
        let (_dir, conn) = conn();
        let suite = suite(&conn);
        let case = insert_cases(&conn, &suite.id, &[draft("a")])
            .expect("插入")
            .remove(0);
        let now = Utc::now();
        let run = TestRun {
            id: "run-1".to_string(),
            suite_id: Some(suite.id.clone()),
            suite_name: suite.name.clone(),
            server_id: "s1".to_string(),
            server_name: "mock".to_string(),
            status: TestRunStatus::Running,
            stop_reason: None,
            options: RunOptions::default(),
            manifest: RunManifest {
                app_version: "test".to_string(),
                server_id: "s1".to_string(),
                server_name: "mock".to_string(),
                protocol_version: "2026-07-28".to_string(),
                lifecycle: "modern".to_string(),
                transport: "stdio".to_string(),
                capability_hash: "hash".to_string(),
                capability_stale: false,
                suite_hash: "suite-hash".to_string(),
                seed: 1,
                tool_count: 1,
                resource_count: 0,
                template_count: 0,
                prompt_count: 0,
                started_at: now,
                finished_at: None,
                options: RunOptions::default(),
            },
            summary: RunSummary {
                total: 1,
                ..Default::default()
            },
            baseline_run_id: None,
            started_at: now,
            finished_at: None,
        };
        create_run(&conn, &run).expect("创建运行");
        insert_run_case(
            &conn,
            &RunCaseRow {
                id: "rc-1".to_string(),
                run_id: "run-1".to_string(),
                case_id: Some(case.id),
                case_name: case.name,
                method: case.method,
                target: case.target,
                order_index: 0,
                case_snapshot: json!({}),
                status: TestRunCaseStatus::Passed,
                failure_kind: None,
                skip_reason: None,
                duration_ms: 12,
                call_record_id: Some("c1".to_string()),
                attempt_record_ids: vec!["c1".to_string()],
                attempts: 1,
                assertions: Vec::new(),
                evidence: Evidence {
                    response_error: Some(crate::mcp::error::CallError::Timeout {
                        timeout_secs: 5,
                    }),
                    ..Default::default()
                },
                message: None,
                created_at: now,
            },
        )
        .expect("写入运行用例");
        let loaded_case = get_run_case(&conn, "rc-1")
            .expect("读取运行用例")
            .expect("存在");
        assert!(matches!(
            loaded_case.evidence.response_error,
            Some(crate::mcp::error::CallError::Timeout { timeout_secs: 5 })
        ));
        let mut summary = run.summary.clone();
        summary.passed = 1;
        update_run_summary(&conn, "run-1", &summary).expect("更新汇总");
        finish_run(&conn, "run-1", TestRunStatus::Completed, None, &summary).expect("收尾");
        let loaded = get_run(&conn, "run-1").expect("读取").expect("存在");
        assert_eq!(loaded.status, TestRunStatus::Completed);
        assert_eq!(loaded.summary.passed, 1);
    }

    #[test]
    fn reorder_rejects_incomplete_list() {
        let (_dir, conn) = conn();
        let suite = suite(&conn);
        let cases = insert_cases(&conn, &suite.id, &[draft("a"), draft("b")]).expect("插入");
        assert!(reorder_cases(&conn, &suite.id, &[cases[0].id.clone()]).is_err());
    }
}
