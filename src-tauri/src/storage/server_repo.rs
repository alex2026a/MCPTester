//! 服务器配置仓储（servers 表 + credentials 索引 + secret_cleanup）
//!
//! 双写一致性（见 04 文档 6.1）：keyring 值为版本化键名（nonce），
//! `credentials` 表为权威索引；`secret_cleanup` 记录删除失败的 keyring 条目供下次启动重试。

use rusqlite::{Connection, OptionalExtension};

use crate::mcp::models::ServerConfig;

/// 服务器行（列表/详情；仅取行级元信息，配置以 config_json 为准）
pub fn list(conn: &Connection) -> anyhow::Result<Vec<ServerConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, config_json, protocol_version_pin, enabled FROM servers ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ServerRow {
            id: row.get(0)?,
            name: row.get(1)?,
            config_json: row.get(2)?,
            protocol_version_pin: row.get(3)?,
            enabled: row.get::<_, i64>(4)? != 0,
        })
    })?;

    let mut servers = Vec::new();
    for row in rows {
        servers.push(row?.into_config()?);
    }
    Ok(servers)
}

/// 单个服务器
pub fn get(conn: &Connection, id: &str) -> anyhow::Result<Option<ServerConfig>> {
    let row = conn
        .query_row(
            "SELECT id, name, config_json, protocol_version_pin, enabled FROM servers WHERE id = ?1",
            [id],
            |row| {
                Ok(ServerRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    config_json: row.get(2)?,
                    protocol_version_pin: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()?;
    match row {
        Some(row) => Ok(Some(row.into_config()?)),
        None => Ok(None),
    }
}

/// 插入或更新（config 为准），并维护 tags/enabled 列
pub fn upsert(conn: &Connection, config: &ServerConfig) -> anyhow::Result<()> {
    let config_json = serde_json::to_string(config)?;
    let tags_json = serde_json::to_string(&config.tags)?;
    conn.execute(
        "INSERT INTO servers (id, name, transport, config_json, protocol_version_pin, enabled,
                              tags_json, created_at, updated_at, last_connected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            transport = excluded.transport,
            config_json = excluded.config_json,
            protocol_version_pin = excluded.protocol_version_pin,
            enabled = excluded.enabled,
            tags_json = excluded.tags_json,
            updated_at = excluded.updated_at",
        rusqlite::params![
            config.id,
            config.name,
            config.transport.as_str(),
            config_json,
            config.protocol_version_pin,
            if config.enabled { 1 } else { 0 },
            tags_json,
            config.created_at.to_rfc3339(),
            config.updated_at.to_rfc3339(),
            config.last_connected_at.map(|t| t.to_rfc3339()),
        ],
    )?;
    Ok(())
}

/// 删除服务器行（credentials 索引由调用方在同一事务中删除）
pub fn delete(conn: &Connection, id: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM servers WHERE id = ?1", [id])?;
    Ok(())
}

/// 更新最后连接时间
pub fn set_last_connected(conn: &Connection, id: &str, at: chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE servers SET last_connected_at = ?2 WHERE id = ?1",
        rusqlite::params![id, at.to_rfc3339()],
    )?;
    Ok(())
}

/// 凭据索引行
#[derive(Debug, Clone)]
pub struct CredentialRef {
    pub server_id: String,
    pub kind: String,
    pub name: String,
    pub reference: String,
}

/// 写入/替换凭据索引（同事务调用）
pub fn upsert_credential(conn: &Connection, item: &CredentialRef) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO credentials (server_id, kind, name, ref, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(server_id, kind, name) DO UPDATE SET ref = excluded.ref, updated_at = excluded.updated_at",
        rusqlite::params![
            item.server_id,
            item.kind,
            item.name,
            item.reference,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// 查询某服务器的全部凭据引用
pub fn list_credentials(conn: &Connection, server_id: &str) -> anyhow::Result<Vec<CredentialRef>> {
    let mut stmt = conn.prepare(
        "SELECT server_id, kind, name, ref FROM credentials WHERE server_id = ?1 ORDER BY kind, name",
    )?;
    let rows = stmt.query_map([server_id], |row| {
        Ok(CredentialRef {
            server_id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            reference: row.get(3)?,
        })
    })?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

/// 全部凭据引用（启动自检用）
pub fn list_all_credentials(conn: &Connection) -> anyhow::Result<Vec<CredentialRef>> {
    let mut stmt =
        conn.prepare("SELECT server_id, kind, name, ref FROM credentials ORDER BY server_id")?;
    let rows = stmt.query_map([], |row| {
        Ok(CredentialRef {
            server_id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            reference: row.get(3)?,
        })
    })?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

/// 删除某服务器的凭据索引
pub fn delete_credentials(conn: &Connection, server_id: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM credentials WHERE server_id = ?1", [server_id])?;
    Ok(())
}

/// 删除单条凭据索引
pub fn delete_credential(conn: &Connection, server_id: &str, kind: &str, name: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM credentials WHERE server_id = ?1 AND kind = ?2 AND name = ?3",
        rusqlite::params![server_id, kind, name],
    )?;
    Ok(())
}

/// 记录 keyring 删除失败项（下次启动重试）
pub fn add_secret_cleanup(conn: &Connection, reference: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO secret_cleanup (ref, created_at) VALUES (?1, ?2)",
        rusqlite::params![reference, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// 取出待清理列表
pub fn list_secret_cleanup(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT ref FROM secret_cleanup")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut refs = Vec::new();
    for row in rows {
        refs.push(row?);
    }
    Ok(refs)
}

/// 移除待清理记录
pub fn remove_secret_cleanup(conn: &Connection, reference: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM secret_cleanup WHERE ref = ?1", [reference])?;
    Ok(())
}

struct ServerRow {
    id: String,
    name: String,
    config_json: String,
    protocol_version_pin: Option<String>,
    enabled: bool,
}

impl ServerRow {
    fn into_config(self) -> anyhow::Result<ServerConfig> {
        // config_json 是权威配置；行级列用于列表排序与快捷查询
        let mut config: ServerConfig = serde_json::from_str(&self.config_json)?;
        config.id = self.id;
        config.name = self.name;
        config.enabled = self.enabled;
        config.protocol_version_pin = self.protocol_version_pin;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{ShellWrap, StdioConfig, TransportKind};
    use chrono::Utc;

    fn temp_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("临时目录创建失败");
        let mut conn = Connection::open(dir.path().join("test.db")).expect("数据库打开失败");
        crate::storage::migrations::run(&mut conn).expect("迁移失败");
        (dir, conn)
    }

    fn sample_config(id: &str) -> ServerConfig {
        let now = Utc::now();
        ServerConfig {
            id: id.to_string(),
            name: format!("mock-{id}"),
            transport: TransportKind::Stdio,
            stdio: Some(StdioConfig {
                command: "mock-mcp-server".to_string(),
                args: vec!["--transport".to_string(), "stdio".to_string()],
                env: Vec::new(),
                cwd: None,
                shell_wrap: ShellWrap::Resolve,
            }),
            http: None,
            protocol_version_pin: None,
            enabled: true,
            tags: vec!["test".to_string()],
            origin: None,
            created_at: now,
            updated_at: now,
            last_connected_at: None,
        }
    }

    #[test]
    fn server_crud_roundtrip() {
        let (_dir, conn) = temp_conn();
        let config = sample_config("s1");
        upsert(&conn, &config).expect("插入失败");

        let loaded = get(&conn, "s1").expect("查询失败").expect("应存在");
        assert_eq!(loaded.name, config.name);
        assert_eq!(loaded.transport, TransportKind::Stdio);
        assert_eq!(loaded.tags, vec!["test".to_string()]);
        assert_eq!(list(&conn).expect("列表失败").len(), 1);

        let mut updated = config.clone();
        updated.name = "renamed".to_string();
        upsert(&conn, &updated).expect("更新失败");
        let loaded = get(&conn, "s1").expect("查询失败").expect("应存在");
        assert_eq!(loaded.name, "renamed");

        delete(&conn, "s1").expect("删除失败");
        assert!(get(&conn, "s1").expect("查询失败").is_none());
    }

    #[test]
    fn credential_index_lifecycle() {
        let (_dir, conn) = temp_conn();
        upsert(&conn, &sample_config("s1")).expect("插入失败");

        let item = CredentialRef {
            server_id: "s1".to_string(),
            kind: "header".to_string(),
            name: "Authorization".to_string(),
            reference: "server/s1/header/Authorization/n1".to_string(),
        };
        upsert_credential(&conn, &item).expect("写入索引失败");
        let items = list_credentials(&conn, "s1").expect("查询失败");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].reference, item.reference);

        // 更新为版本化新 nonce
        let updated = CredentialRef {
            reference: "server/s1/header/Authorization/n2".to_string(),
            ..item
        };
        upsert_credential(&conn, &updated).expect("更新索引失败");
        let items = list_credentials(&conn, "s1").expect("查询失败");
        assert_eq!(items[0].reference, "server/s1/header/Authorization/n2");

        delete_credentials(&conn, "s1").expect("删除索引失败");
        assert!(list_credentials(&conn, "s1").expect("查询失败").is_empty());
    }

    #[test]
    fn secret_cleanup_queue() {
        let (_dir, conn) = temp_conn();
        add_secret_cleanup(&conn, "ref-a").expect("入队失败");
        add_secret_cleanup(&conn, "ref-a").expect("重复入队应幂等");
        add_secret_cleanup(&conn, "ref-b").expect("入队失败");
        let mut refs = list_secret_cleanup(&conn).expect("查询失败");
        refs.sort();
        assert_eq!(refs, vec!["ref-a".to_string(), "ref-b".to_string()]);
        remove_secret_cleanup(&conn, "ref-a").expect("移除失败");
        assert_eq!(list_secret_cleanup(&conn).expect("查询失败").len(), 1);
    }
}
