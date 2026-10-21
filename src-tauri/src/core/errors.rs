//! 应用统一错误类型（M2 起扩展 McpError / CallError，见设计文档 04 第 1 节）

// M1 脚手架：部分变体在 M2 仓储/命令层接入后被构造，届时移除此 allow
#![allow(dead_code)]

/// 应用层错误
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 平台凭据库初始化失败
    #[error("凭据存储不可用：{0}")]
    CredentialStoreUnavailable(String),

    /// 凭据读写失败
    #[error("凭据操作失败：{0}")]
    Credential(String),

    /// 凭据不存在（keyring 无对应条目）
    #[error("凭据不存在，请重新配置")]
    CredentialNotFound,

    /// 数据库错误
    #[error("数据库错误：{0}")]
    Database(String),

    /// IO 错误
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
}
