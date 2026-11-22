//! 压测运行仓储（v0.7.0）。

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::loadtest::models::{
    LoadErrorSample, LoadRun, LoadRunStatus, LoadRunSummary, LoadSeriesPoint, LoadSummary,
};

pub fn insert_run(conn: &Connection, run: &LoadRun) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO load_runs
            (id, server_id, server_name, suite_id, suite_name, profile_name, status,
             stop_reason, profile_json, operations_json, manifest_json, summary_json,
             series_json, errors_json, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            run.id,
            run.server_id,
            run.server_name,
            run.suite_id,
            run.suite_name,
            run.profile_name,
            run.status.as_str(),
            run.stop_reason,
            serde_json::to_string(&run.profile)?,
            serde_json::to_string(&run.operations)?,
            serde_json::to_string(&run.manifest)?,
            serde_json::to_string(&run.summary)?,
            serde_json::to_string(&run.series)?,
            serde_json::to_string(&run.errors)?,
            run.started_at.to_rfc3339(),
            run.finished_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    status: LoadRunStatus,
    stop_reason: Option<&str>,
    summary: &LoadSummary,
    series: &[LoadSeriesPoint],
    errors: &[LoadErrorSample],
    finished_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let changed = conn.execute(
        "UPDATE load_runs
            SET status = ?1, stop_reason = ?2, summary_json = ?3, series_json = ?4,
                errors_json = ?5, finished_at = ?6
          WHERE id = ?7",
        rusqlite::params![
            status.as_str(),
            stop_reason,
            serde_json::to_string(summary)?,
            serde_json::to_string(series)?,
            serde_json::to_string(errors)?,
            finished_at.to_rfc3339(),
            run_id,
        ],
    )?;
    if changed == 0 {
        anyhow::bail!("压测运行不存在");
    }
    Ok(())
}

pub fn get_run(conn: &Connection, id: &str) -> anyhow::Result<Option<LoadRun>> {
    let row = conn
        .query_row(
            "SELECT id, server_id, server_name, suite_id, suite_name, profile_name, status,
                    stop_reason, profile_json, operations_json, manifest_json, summary_json,
                    series_json, errors_json, started_at, finished_at
               FROM load_runs WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, Option<String>>(15)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        server_id,
        server_name,
        suite_id,
        suite_name,
        profile_name,
        status,
        stop_reason,
        profile_json,
        operations_json,
        manifest_json,
        summary_json,
        series_json,
        errors_json,
        started_at,
        finished_at,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(LoadRun {
        id,
        server_id,
        server_name,
        suite_id,
        suite_name,
        profile_name,
        status: LoadRunStatus::parse(&status)
            .ok_or_else(|| anyhow::anyhow!("未知压测状态：{status}"))?,
        stop_reason,
        profile: parse_json(&profile_json, "压测 profile")?,
        operations: parse_json(&operations_json, "压测操作快照")?,
        manifest: parse_json(&manifest_json, "压测 manifest")?,
        summary: parse_json(&summary_json, "压测 summary")?,
        series: parse_json(&series_json, "压测 series")?,
        errors: parse_json(&errors_json, "压测 errors")?,
        started_at: parse_time(&started_at)?,
        finished_at: finished_at.map(|value| parse_time(&value)).transpose()?,
    }))
}

pub fn list_runs(
    conn: &Connection,
    server_id: Option<&str>,
    offset: u64,
    limit: u32,
) -> anyhow::Result<(Vec<LoadRunSummary>, u64)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM load_runs WHERE (?1 IS NULL OR server_id = ?1)",
        [server_id],
        |row| row.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT id, server_id, server_name, suite_id, suite_name, profile_name, status,
                stop_reason, summary_json, started_at, finished_at
           FROM load_runs
          WHERE (?1 IS NULL OR server_id = ?1)
          ORDER BY started_at DESC
          LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![server_id, limit as i64, offset as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (
            id,
            server_id,
            server_name,
            suite_id,
            suite_name,
            profile_name,
            status,
            stop_reason,
            summary_json,
            started_at,
            finished_at,
        ) = row?;
        let summary: LoadSummary = serde_json::from_str(&summary_json).unwrap_or_default();
        items.push(LoadRunSummary {
            id,
            server_id,
            server_name,
            suite_id,
            suite_name,
            profile_name,
            status: LoadRunStatus::parse(&status)
                .ok_or_else(|| anyhow::anyhow!("未知压测状态：{status}"))?,
            stop_reason,
            total: summary.total,
            success_rate: summary.success_rate,
            rps: summary.rps,
            p95_ms: summary.p95_ms,
            p99_ms: summary.p99_ms,
            duration_ms: summary.duration_ms,
            max_vus: summary.max_vus,
            started_at: parse_time(&started_at)?,
            finished_at: finished_at.map(|value| parse_time(&value)).transpose()?,
        });
    }
    Ok((items, total as u64))
}

pub fn recover_interrupted_runs(conn: &Connection) -> anyhow::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let changed = conn.execute(
        "UPDATE load_runs SET status = 'interrupted', stop_reason = 'interrupted',
                finished_at = ?1 WHERE status = 'running'",
        [now],
    )?;
    Ok(changed as u64)
}

pub fn cleanup_runs(conn: &Connection, retention_days: u32, max_rows: u32) -> anyhow::Result<u64> {
    let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
    let mut ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM load_runs WHERE started_at < ?1")?;
        stmt.query_map([&cutoff], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect()
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM load_runs", [], |row| row.get(0))?;
    let overflow = total - max_rows as i64;
    if overflow > 0 {
        let mut stmt = conn.prepare("SELECT id FROM load_runs ORDER BY started_at ASC LIMIT ?1")?;
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
    let mut deleted = 0u64;
    for id in ids {
        deleted += conn.execute("DELETE FROM load_runs WHERE id = ?1", [&id])? as u64;
    }
    Ok(deleted)
}

pub fn export_json(run: &LoadRun) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(run)?)
}

fn parse_json<T: serde::de::DeserializeOwned>(text: &str, label: &str) -> anyhow::Result<T> {
    serde_json::from_str(text).map_err(|error| anyhow::anyhow!("{label} JSON 解析失败：{error}"))
}

fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .map_err(|error| anyhow::anyhow!("时间解析失败：{error}"))?
        .with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadtest::models::{
        LoadCaseSnapshot, LoadManifest, LoadOperation, LoadProfile, LoadRiskSummary, LoadRunStatus,
        LoadSessionMode, LoadStage,
    };
    use crate::testing::models::{TestMethod, TestRisk};
    use serde_json::json;

    fn temp_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let mut conn =
            Connection::open(dir.path().join("test.db")).expect("数据库打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");
        (dir, conn)
    }

    fn sample_run(id: &str, status: LoadRunStatus) -> LoadRun {
        let now = Utc::now();
        LoadRun {
            id: id.to_string(),
            server_id: "s1".to_string(),
            server_name: "demo".to_string(),
            suite_id: "suite-1".to_string(),
            suite_name: "套件".to_string(),
            profile_name: "profile".to_string(),
            status,
            stop_reason: None,
            profile: LoadProfile {
                name: "profile".to_string(),
                session_mode: LoadSessionMode::PerVu,
                stages: vec![LoadStage {
                    duration_secs: 5,
                    target_vus: 2,
                }],
                think_time_min_ms: 0,
                think_time_max_ms: 0,
                timeout_secs: 5,
                operations: vec![LoadOperation {
                    case_id: "c1".to_string(),
                    weight: 1,
                }],
                allow_write: false,
                allow_unknown: false,
                abort_error_rate: Some(0.2),
                abort_p99_ms: None,
                seed: 1,
            },
            operations: vec![LoadCaseSnapshot {
                case_id: "c1".to_string(),
                name: "echo".to_string(),
                method: TestMethod::ToolsCall,
                target: "echo".to_string(),
                arguments: json!({ "text": "hi" }),
                risk: TestRisk::ReadOnly,
                weight: 1,
                timeout_secs: None,
            }],
            manifest: LoadManifest {
                app_version: "0.7.0".to_string(),
                server_id: "s1".to_string(),
                server_name: "demo".to_string(),
                transport: "stdio".to_string(),
                protocol_version: "2026-07-28".to_string(),
                lifecycle: "modern".to_string(),
                capability_hash: "hash".to_string(),
                capability_stale: false,
                tool_count: 1,
                resource_count: 0,
                template_count: 0,
                prompt_count: 0,
                production: false,
                risk_summary: LoadRiskSummary {
                    read_only: 1,
                    ..Default::default()
                },
                started_at: now,
            },
            summary: LoadSummary {
                total: 2,
                eligible: 2,
                ok: 2,
                success_rate: 1.0,
                duration_ms: 1000,
                rps: 2.0,
                ..Default::default()
            },
            series: vec![LoadSeriesPoint {
                at_ms: 1000,
                target_vus: 2,
                eligible_delta: 2,
                rps: 2.0,
                ..Default::default()
            }],
            errors: vec![],
            started_at: now,
            finished_at: Some(now),
        }
    }

    #[test]
    fn insert_get_list_finish_and_export() {
        let (_dir, conn) = temp_db();
        let mut run = sample_run("run-1", LoadRunStatus::Running);
        run.finished_at = None;
        insert_run(&conn, &run).expect("插入失败");
        let loaded = get_run(&conn, "run-1").expect("读取失败").expect("不存在");
        assert_eq!(loaded.summary.ok, 2);
        let (items, total) = list_runs(&conn, None, 0, 10).expect("列表失败");
        assert_eq!(total, 1);
        assert_eq!(items[0].id, "run-1");
        assert_eq!(items[0].success_rate, 1.0);

        let summary = LoadSummary {
            total: 3,
            ok: 3,
            eligible: 3,
            success_rate: 1.0,
            duration_ms: 2000,
            ..Default::default()
        };
        finish_run(
            &conn,
            "run-1",
            LoadRunStatus::Completed,
            None,
            &summary,
            &[],
            &[],
            Utc::now(),
        )
        .expect("完成失败");
        let loaded = get_run(&conn, "run-1").expect("读取失败").expect("不存在");
        assert_eq!(loaded.status, LoadRunStatus::Completed);
        assert_eq!(loaded.summary.total, 3);
        let json = export_json(&loaded).expect("导出失败");
        assert!(json.contains("\"status\": \"completed\""));
        assert!(json.contains("\"operations\""));
    }

    #[test]
    fn recover_and_cleanup() {
        let (_dir, conn) = temp_db();
        insert_run(&conn, &sample_run("run-old", LoadRunStatus::Running)).expect("插入失败");
        assert_eq!(recover_interrupted_runs(&conn).expect("回收失败"), 1);
        let run = get_run(&conn, "run-old").expect("读取失败").expect("不存在");
        assert_eq!(run.status, LoadRunStatus::Interrupted);
        assert_eq!(cleanup_runs(&conn, 0, 0).expect("清理失败"), 1);
        assert!(get_run(&conn, "run-old").expect("读取失败").is_none());
    }
}
