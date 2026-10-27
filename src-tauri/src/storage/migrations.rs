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
    (4, MIGRATION_0004),
    (5, MIGRATION_0005),
    (6, MIGRATION_0006),
    (7, MIGRATION_0007),
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

/// 迁移 0004：服务器自定义排序（v0.2.3）
///
/// 按 created_at, id 回填，保证升级后的顺序与旧版 `ORDER BY created_at ASC` 一致。
/// SQLite 的 ALTER TABLE 不支持 IF NOT EXISTS；迁移与版本号在同一事务内提交，
/// 失败时整体回滚，因此不会留下“列已存在但版本未更新”的半成品状态。
const MIGRATION_0004: &str = r#"
ALTER TABLE servers ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;

UPDATE servers SET sort_order = (
    SELECT COUNT(*) FROM servers AS earlier
    WHERE earlier.created_at < servers.created_at
       OR (earlier.created_at = servers.created_at AND earlier.id < servers.id)
);

CREATE INDEX IF NOT EXISTS idx_servers_sort_order ON servers(sort_order);
"#;

/// 迁移 0005：OAuth 2.1 登录（v0.3.0）
///
/// 只新增 `oauth_credentials`，不重建 `credentials`。refresh token 与
/// client_secret 的安装版 ref / 便携版明文都在这张表管理。
const MIGRATION_0005: &str = r#"
CREATE TABLE IF NOT EXISTS oauth_credentials (
    server_id TEXT PRIMARY KEY NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    refresh_ref TEXT,
    refresh_token TEXT,
    refresh_chunks INTEGER NOT NULL DEFAULT 0 CHECK (refresh_chunks >= 0),
    client_id TEXT,
    client_secret_ref TEXT,
    client_secret TEXT,
    client_secret_source TEXT CHECK (client_secret_source IN ('user','dcr')),
    client_secret_version INTEGER NOT NULL DEFAULT 0 CHECK (client_secret_version >= 0),
    redirect_uri TEXT,
    scopes_json TEXT NOT NULL DEFAULT '[]',
    issuer TEXT,
    expires_at TEXT,
    has_token INTEGER NOT NULL DEFAULT 0 CHECK (has_token IN (0,1)),
    has_refresh_token INTEGER NOT NULL DEFAULT 0 CHECK (has_refresh_token IN (0,1)),
    config_fingerprint TEXT,
    updated_at TEXT NOT NULL,
    CHECK (refresh_ref IS NULL OR refresh_token IS NULL),
    CHECK (client_secret_ref IS NULL OR client_secret IS NULL)
);
CREATE INDEX IF NOT EXISTS idx_oauth_credentials_ref ON oauth_credentials(refresh_ref);
"#;

/// 迁移 0006：OAuth 注册方式（v0.3.1）
///
/// SQLite 的 `ALTER TABLE ADD COLUMN` 不能带复杂约束；列值由 Rust 枚举校验。
const MIGRATION_0006: &str = r#"
ALTER TABLE oauth_credentials ADD COLUMN registration_kind TEXT;
UPDATE oauth_credentials
   SET registration_kind = 'dcr'
 WHERE client_secret_source = 'dcr';
UPDATE oauth_credentials
   SET registration_kind = 'preregistered'
 WHERE client_secret_source = 'user';
UPDATE oauth_credentials
   SET registration_kind = 'preregistered'
 WHERE client_secret_source IS NULL AND client_id IS NOT NULL AND has_token = 1;
"#;

/// 迁移 0007：原始 JSON-RPC 帧（v0.5.0）
///
/// 四列均为可选；旧记录保持 NULL/0。`rpc_id` 以 JSON 字面量保存，
/// 保留线上 number/string 类型。
const MIGRATION_0007: &str = r#"
ALTER TABLE call_records ADD COLUMN rpc_id TEXT;
ALTER TABLE call_records ADD COLUMN request_frame_json TEXT;
ALTER TABLE call_records ADD COLUMN response_frame_json TEXT;
ALTER TABLE call_records ADD COLUMN frames_truncated INTEGER NOT NULL DEFAULT 0;
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
        assert_eq!(version, 7);

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
    fn upgrade_v2_keeps_call_records_and_sets_manual_origin() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(MIGRATION_0001).expect("0001 失败");
        conn.execute_batch(MIGRATION_0002).expect("0002 失败");
        conn.pragma_update(None, "user_version", 2)
            .expect("版本设置失败");
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
            .query_row(
                "SELECT origin FROM call_records WHERE id = 'c1'",
                [],
                |row| row.get(0),
            )
            .expect("记录丢失");
        assert_eq!(origin, "manual");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
    }

    #[test]
    fn upgrade_v3_to_v4_backfills_sort_order() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(MIGRATION_0001).expect("0001 失败");
        conn.execute_batch(MIGRATION_0002).expect("0002 失败");
        conn.execute_batch(MIGRATION_0003).expect("0003 失败");
        conn.pragma_update(None, "user_version", 3)
            .expect("版本设置失败");

        // 插入顺序与 created_at 顺序不同，验证按时间回填
        for (id, created_at) in [
            ("s3", "2026-01-03T00:00:00Z"),
            ("s1", "2026-01-01T00:00:00Z"),
            ("s2", "2026-01-02T00:00:00Z"),
        ] {
            conn.execute(
                "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
                 VALUES (?1, ?1, 'stdio', '{}', ?2, ?2)",
                rusqlite::params![id, created_at],
            )
            .expect("服务器插入失败");
        }

        run(&mut conn).expect("升级失败");
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM servers ORDER BY sort_order ASC, name ASC")
            .expect("查询准备失败")
            .query_map([], |row| row.get(0))
            .expect("查询失败")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("读取失败");
        assert_eq!(ids, vec!["s1", "s2", "s3"]);
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
    }

    #[test]
    fn upgrade_v4_to_v5_creates_oauth_table() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        conn.execute_batch(MIGRATION_0001).expect("0001 失败");
        conn.execute_batch(MIGRATION_0002).expect("0002 失败");
        conn.execute_batch(MIGRATION_0003).expect("0003 失败");
        conn.execute_batch(MIGRATION_0004).expect("0004 失败");
        conn.pragma_update(None, "user_version", 4)
            .expect("版本设置失败");
        conn.execute(
            "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
             VALUES ('s1', 'demo', 'streamable_http', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("服务器插入失败");
        conn.execute(
            "INSERT INTO credentials (server_id, kind, name, ref, updated_at)
             VALUES ('s1', 'header', 'Authorization', 'ref-1', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("凭据插入失败");

        run(&mut conn).expect("升级失败");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
        let ref_value: String = conn
            .query_row(
                "SELECT ref FROM credentials WHERE server_id = 's1' AND kind = 'header'",
                [],
                |row| row.get(0),
            )
            .expect("旧凭据丢失");
        assert_eq!(ref_value, "ref-1");
        let oauth_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='oauth_credentials'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(oauth_table, 1);
    }

    #[test]
    fn upgrade_v5_to_v6_backfills_registration_kind() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        for sql in [
            MIGRATION_0001,
            MIGRATION_0002,
            MIGRATION_0003,
            MIGRATION_0004,
            MIGRATION_0005,
        ] {
            conn.execute_batch(sql).expect("迁移失败");
        }
        conn.pragma_update(None, "user_version", 5)
            .expect("版本设置失败");
        for id in ["s1", "s2", "s3"] {
            conn.execute(
                "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
                 VALUES (?1, ?1, 'streamable_http', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [id],
            )
            .expect("服务器插入失败");
        }
        for (id, source, client_id) in [
            ("s1", Some("dcr"), Some("client-1")),
            ("s2", Some("user"), Some("client-2")),
            ("s3", None, Some("client-3")),
        ] {
            conn.execute(
                "INSERT INTO oauth_credentials
                    (server_id, client_id, client_secret_source, has_token, has_refresh_token, updated_at)
                 VALUES (?1, ?2, ?3, 1, 0, '2026-01-01T00:00:00Z')",
                rusqlite::params![id, client_id, source],
            )
            .expect("OAuth 行插入失败");
        }

        run(&mut conn).expect("升级失败");
        let kinds: Vec<Option<String>> = conn
            .prepare("SELECT registration_kind FROM oauth_credentials ORDER BY server_id")
            .expect("查询准备失败")
            .query_map([], |row| row.get(0))
            .expect("查询失败")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("读取失败");
        assert_eq!(
            kinds,
            vec![
                Some("dcr".to_string()),
                Some("preregistered".to_string()),
                Some("preregistered".to_string())
            ]
        );
    }

    #[test]
    fn upgrade_v6_to_v7_adds_frame_columns_and_keeps_old_record() {
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        for sql in [
            MIGRATION_0001,
            MIGRATION_0002,
            MIGRATION_0003,
            MIGRATION_0004,
            MIGRATION_0005,
            MIGRATION_0006,
        ] {
            conn.execute_batch(sql).expect("迁移失败");
        }
        conn.pragma_update(None, "user_version", 6)
            .expect("版本设置失败");
        conn.execute(
            "INSERT INTO call_records
                (id, server_id, server_name, method, target, params_json, status, duration_ms,
                 protocol_version, transport, size_bytes, origin, created_at)
             VALUES ('old-1', 's1', 'demo', 'tools_call', 'echo', '{}', 'ok', 1,
                     '2026-07-28', 'stdio', 0, 'manual', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("旧记录插入失败");

        run(&mut conn).expect("升级失败");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);

        let row: (String, Option<String>, Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT id, rpc_id, request_frame_json, response_frame_json, frames_truncated
                   FROM call_records WHERE id = 'old-1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("旧记录丢失");
        assert_eq!(row.0, "old-1");
        assert_eq!(row.1, None);
        assert_eq!(row.2, None);
        assert_eq!(row.3, None);
        assert_eq!(row.4, 0);
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
            7
        );
    }
}
