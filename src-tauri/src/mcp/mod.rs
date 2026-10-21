//! MCP 客户端核心（M2 实现）
//!
//! - `models`：领域模型与前端契约（04 文档第 1 节）
//! - `error`：连接级 / 调用级错误分类（04 文档第 5 节）
//! - `handler`：通知扇入（ClientHandler 回调 + Subscription 两条来源 → EventHub）
//! - `transport/`：传输工厂与 stdio / Streamable HTTP / 遗留 SSE 三实现
//! - `connection`：单服务器连接（生命周期 / 能力枚举 / 调用管线）
//! - `manager`：多服务器连接管理
//! - `wire`：JSON-RPC 线上报文日志（v0.1.1，内存环形缓冲 + 脱敏）

pub mod connection;
pub mod error;
pub mod handler;
pub mod manager;
pub mod models;
pub mod transport;
pub mod wire;

pub use manager::ConnectionManager;
