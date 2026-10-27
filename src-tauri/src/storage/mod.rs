//! 存储模块：SQLite 句柄、版本化迁移与仓储（见设计文档 04）

pub mod app_meta;
pub mod call_repo;
pub mod db;
pub mod migrations;
pub mod oauth_repo;
pub mod server_repo;
pub mod settings_repo;
pub mod test_repo;

pub use db::Database;
