//! 应用内部状态表（非用户设置）：启动配置 seed 状态等。
//!
//! 与 `settings` 分离，避免前端保存设置时覆盖内部字段。

use rusqlite::{Connection, OptionalExtension};

/// 读取内部状态值
pub fn get(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    let value = conn
        .query_row("SELECT value FROM app_meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value)
}

/// 写入/替换内部状态值
pub fn set(conn: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_roundtrip() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("内存库打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");

        assert!(get(&conn, "bootstrap").expect("读取失败").is_none());
        set(&conn, "bootstrap", r#"{"applied":{}}"#).expect("写入失败");
        assert_eq!(
            get(&conn, "bootstrap").expect("读取失败").as_deref(),
            Some(r#"{"applied":{}}"#)
        );
        set(&conn, "bootstrap", r#"{"applied":{"a":"b"}}"#).expect("更新失败");
        assert_eq!(
            get(&conn, "bootstrap").expect("读取失败").as_deref(),
            Some(r#"{"applied":{"a":"b"}}"#)
        );
    }
}
