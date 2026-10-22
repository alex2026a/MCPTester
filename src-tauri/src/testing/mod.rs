//! v0.2.1 单步契约测试：领域模型、生成器、断言器、运行器与报告。
//!
//! 模块划分见各模块注释。

pub mod assertions;
pub mod contract;
pub mod coverage;
pub mod generator;
pub mod history;
pub mod llm;
pub mod models;
pub mod report;
pub mod runner;
pub mod schema;

pub use models::*;
