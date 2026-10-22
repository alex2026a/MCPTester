//! 版本化数据库迁移（以 `PRAGMA user_version` 记录 schema 版本）
//!
//! 规则（见设计文档 04 第 3 节）：
//! - 每个迁移先做表/列/索引存在性检查再创建（IF NOT EXISTS，幂等）；
//! - 迁移与版本号更新在同一事务内提交；
//! - 新增 schema 变更时追加迁移条目，禁止修改历史迁移。

use rusqlite::Connection;

/// 迁移列表：(目标版本, SQL)
const MIGRATIONS: &[(i64, &str)] = &[
    (1, MIGRATION_0001),
    (2, MIGRATION_0002),
    (3, MIGRATION_0003),
];

/// 执行所有未应用的迁移
pub fn run(conn: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (version, sql) in MIGRATIONS {
        if current < *version {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
            tracing::info!(version, "数据库迁移完成");
        }
    }
    Ok(())
}

/// 迁移 0001：初始 schema（servers / credentials / secret_cleanup / call_records / settings）
///
/// 对应设计文档 04 第 3 节；跨表删除以应用层同事务为准（连接创建即 foreign_keys=ON）。
const MIGRATION_0001: &str = r#"
CREATE TABLE IF NOT EXISTS servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    transport TEXT NOT NULL CHECK (transport IN ('stdio','streamable_http','sse')),
    config_json TEXT NOT NULL,
    protocol_version_pin TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    tags_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_connected_at TEXT
);

CREATE TABLE IF NOT EXISTS credentials (
    server_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('env','header')),
    name TEXT NOT NULL,
    ref TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (server_id, kind, name)
);
CREATE INDEX IF NOT EXISTS idx_credentials_ref ON credentials(ref);

CREATE TABLE IF NOT EXISTS secret_cleanup (
    ref TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS call_records (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL,
    server_name TEXT NOT NULL,
    method TEXT NOT NULL CHECK (method IN ('tools_call','resources_read','prompts_get','probe')),
    target TEXT NOT NULL,
    params_json TEXT NOT NULL,
    result_json TEXT,
    error_json TEXT,
    status TEXT NOT NULL CHECK (status IN ('ok','error','canceled')),
    duration_ms INTEGER NOT NULL,
    protocol_version TEXT NOT NULL,
    transport TEXT NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_call_records_server_time ON call_records(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_call_records_method       ON call_records(method);
CREATE INDEX IF NOT EXISTS idx_call_records_status       ON call_records(status);
CREATE INDEX IF NOT EXISTS idx_call_records_created      ON call_records(created_at);

CREATE TABLE IF NOT EXISTS settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);
"#;

/// 迁移 0002：应用内部状态（启动配置 seed 标记、文件哈希等）
const MIGRATION_0002: &str = r#"
CREATE TABLE IF NOT EXISTS app_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// 迁移 0003：v0.2.1 单步契约测试（套件/用例/运行/结果 + 调用来源）
const MIGRATION_0003: &str = r#"
CREATE TABLE IF NOT EXISTS test_suites (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    server_id TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    capability_hash TEXT,
    capability_snapshot_json TEXT,
    baseline_run_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS test_cases (
    id TEXT PRIMARY KEY,
    suite_id TEXT NOT NULL REFERENCES test_suites(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL CHECK (method IN ('tools_call','resources_read','prompts_get')),
    target TEXT NOT NULL,
    arguments_json TEXT NOT NULL DEFAULT '{}',
    expect_json TEXT NOT NULL,
    assertions_json TEXT NOT NULL DEFAULT '[]',
    tags_json TEXT NOT NULL DEFAULT '[]',
    priority TEXT NOT NULL CHECK (priority IN ('p0','p1','p2')),
    risk TEXT NOT NULL CHECK (risk IN ('read_only','write','unknown','destructive')),
    source TEXT NOT NULL CHECK (source IN ('manual','schema','history','llm','import')),
    generation_note TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    order_index INTEGER NOT NULL DEFAULT 0,
    timeout_secs INTEGER,
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_test_cases_suite_order ON test_cases(suite_id, order_index, id);

CREATE TABLE IF NOT EXISTS test_runs (
    id TEXT PRIMARY KEY,
    suite_id TEXT,
    suite_name TEXT NOT NULL,
    server_id TEXT NOT NULL,
    server_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running','completed','failed','canceled','interrupted')),
    stop_reason TEXT,
    options_json TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    baseline_run_id TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_test_runs_started ON test_runs(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_test_runs_suite ON test_runs(suite_id, started_at DESC);

CREATE TABLE IF NOT EXISTS test_run_cases (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
    case_id TEXT,
    case_name TEXT NOT NULL,
    method TEXT NOT NULL,
    target TEXT NOT NULL,
    order_index INTEGER NOT NULL DEFAULT 0,
    case_snapshot_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('passed','failed','warning','skipped','error','canceled')),
    failure_kind TEXT,
    skip_reason TEXT,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    call_record_id TEXT,
    attempt_record_ids_json TEXT NOT NULL DEFAULT '[]',
    attempts INTEGER NOT NULL DEFAULT 1,
    assertions_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    message TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_test_run_cases_run ON test_run_cases(run_id, order_index, id);

ALTER TABLE call_records ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual'
    CHECK (origin IN ('manual','test'));
CREATE INDEX IF NOT EXISTS idx_call_records_origin_time
    ON call_records(origin, server_id, created_at DESC);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migration_0001_creates_schema() {
        // 全部迁移在空库上应用成功，版本号与表齐备
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        run(&mut conn).expect("迁移失败");

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);

        // transport 约束生效（非法值应失败）
        let insert_err = conn.execute(
            "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
             VALUES ('s1', 'demo', 'invalid', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        );
        assert!(insert_err.is_err(), "非法 transport 应被约束拒绝");

        // method 约束生效
        let method_err = conn.execute(
            "INSERT INTO call_records (id, server_id, server_name, method, target, params_json, status, duration_ms, protocol_version, transport, size_bytes, created_at)
             VALUES ('c1', 's1', 'demo', 'ping', 'probe', '{}', 'ok', 1, '2026-07-28', 'stdio', 2, '2026-01-01T00:00:00Z')",
            [],
        );
        assert!(method_err.is_err(), "旧 method（ping）应被约束拒绝");
    }

    #[test]
    fn upgrade_v2_to_v3_keeps_call_records_and_sets_manual_origin() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(MIGRATION_0001).expect("0001 失败");
        conn.execute_batch(MIGRATION_0002).expect("0002 失败");
        conn.pragma_update(None, "user_version", 2).expect("版本设置失败");
        conn.execute(
            "INSERT INTO call_records
                (id, server_id, server_name, method, target, params_json, status, duration_ms,
                 protocol_version, transport, size_bytes, created_at)
             VALUES ('c1', 's1', 'demo', 'tools_call', 'echo', '{}', 'ok', 1, '2026-07-28',
                     'stdio', 0, '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("旧记录插入失败");

        run(&mut conn).expect("升级失败");
        let origin: String = conn
            .query_row("SELECT origin FROM call_records WHERE id = 'c1'", [], |row| {
                row.get(0)
            })
            .expect("记录丢失");
        assert_eq!(origin, "manual");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn migration_is_idempotent() {
        // 重复执行不报错，且不破坏已有数据
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        run(&mut conn).expect("第一次迁移失败");
        run(&mut conn).expect("第二次迁移失败");
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}
