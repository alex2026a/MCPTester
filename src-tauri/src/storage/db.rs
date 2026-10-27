//! 数据库句柄：单写连接 + 单读连接（见设计文档 03 第 6.3 节）
//!
//! - rusqlite 为同步 API，全部调用经 `tokio::task::spawn_blocking` 包装；
//! - 初始化启用 WAL、busy_timeout 与 foreign_keys；
//! - 迁移以 `PRAGMA user_version` 版本化管理，启动时单线程执行。

// M1 脚手架：异步读写包装器在 M2 仓储层接入后被调用，届时移除此 allow
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::migrations;

/// SQLite 数据库句柄
#[derive(Clone)]
pub struct Database {
    write: Arc<Mutex<Connection>>,
    read: Arc<Mutex<Connection>>,
}

impl Database {
    /// 打开（或创建）数据库，初始化连接参数
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let write = Self::connect(path)?;
        let read = Self::connect(path)?;
        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read: Arc::new(Mutex::new(read)),
        })
    }

    fn connect(path: &Path) -> anyhow::Result<Connection> {
        let conn = Connection::open(path)?;
        // journal_mode 返回一行结果，用 query_row 执行
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(conn)
    }

    /// 执行版本化迁移（启动时单线程调用）
    pub fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self
            .write
            .lock()
            .map_err(|_| anyhow::anyhow!("数据库写锁异常"))?;
        migrations::run(&mut conn)
    }

    /// 阻塞式写操作（启动自检等同步场景使用，勿在 async 上下文调用）
    pub fn with_write_blocking<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Connection) -> anyhow::Result<T>,
    {
        let mut guard = self
            .write
            .lock()
            .map_err(|_| anyhow::anyhow!("数据库写锁异常"))?;
        f(&mut guard)
    }

    /// 阻塞式读操作（查询短小的同步场景使用）
    pub fn with_read_blocking<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Connection) -> anyhow::Result<T>,
    {
        let guard = self
            .read
            .lock()
            .map_err(|_| anyhow::anyhow!("数据库读锁异常"))?;
        f(&guard)
    }

    /// 在阻塞线程池中执行写操作
    pub async fn with_write<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.write);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().map_err(|_| anyhow::anyhow!("数据库写锁异常"))?;
            f(&mut guard)
        })
        .await?
    }

    /// 在阻塞线程池中执行只读查询
    pub async fn with_read<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.read);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|_| anyhow::anyhow!("数据库读锁异常"))?;
            f(&guard)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let db = Database::open(&dir.path().join("test.db")).expect("数据库打开失败");
        (dir, db)
    }

    #[test]
    fn migrate_sets_user_version() {
        // 迁移后 user_version = 最新版本（当前为 7），且关键表存在
        let (_dir, db) = temp_db();
        db.migrate().expect("迁移失败");

        let conn = db.write.lock().unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);

        for table in [
            "servers",
            "credentials",
            "secret_cleanup",
            "call_records",
            "settings",
            "app_meta",
            "test_suites",
            "test_cases",
            "test_runs",
            "test_run_cases",
            "oauth_credentials",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "表 {table} 不存在");
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        // 迁移重复执行不报错（幂等）
        let (_dir, db) = temp_db();
        db.migrate().expect("第一次迁移失败");
        db.migrate().expect("第二次迁移失败");
    }

    #[test]
    fn foreign_keys_enabled() {
        // 外键约束已开启
        let (_dir, db) = temp_db();
        let conn = db.write.lock().unwrap();
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        // WAL 模式已启用
        let (_dir, db) = temp_db();
        let conn = db.write.lock().unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[tokio::test]
    async fn async_read_write_wrappers() {
        // 异步读写包装器可用
        let (_dir, db) = temp_db();
        db.migrate().unwrap();

        db.with_write(|conn| {
            conn.execute(
                "INSERT INTO settings (id, json) VALUES (1, ?1)",
                ["{\"request_timeout_secs\":30}"],
            )?;
            Ok(())
        })
        .await
        .expect("异步写入失败");

        let value: String = db
            .with_read(|conn| {
                let v = conn.query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                    row.get(0)
                })?;
                Ok(v)
            })
            .await
            .expect("异步读取失败");
        assert!(value.contains("30"));
    }
}
