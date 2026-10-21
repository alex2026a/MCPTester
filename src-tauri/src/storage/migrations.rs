//! 版本化数据库迁移（以 `PRAGMA user_version` 记录 schema 版本）
//!
//! 规则（见设计文档 04 第 3 节）：
//! - 每个迁移先做表/列/索引存在性检查再创建（IF NOT EXISTS，幂等）；
//! - 迁移与版本号更新在同一事务内提交；
//! - 新增 schema 变更时追加迁移条目，禁止修改历史迁移。

use rusqlite::Connection;

/// 迁移列表：(目标版本, SQL)
const MIGRATIONS: &[(i64, &str)] = &[(1, MIGRATION_0001), (2, MIGRATION_0002)];

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
        assert_eq!(version, 2);

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
    fn migration_is_idempotent() {
        // 重复执行不报错，且不破坏已有数据
        let mut conn = Connection::open_in_memory().expect("内存库打开失败");
        run(&mut conn).expect("第一次迁移失败");
        run(&mut conn).expect("第二次迁移失败");
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
