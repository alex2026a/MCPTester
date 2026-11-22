//! Tauri command 层（按 server / capability / call / events / system 拆分）

pub mod bootstrap;
pub mod call;
pub mod capability;
pub mod capability_export;
pub mod events;
pub mod history;
pub mod import_export;
pub mod input;
pub mod loadtest;
pub mod oauth;
pub mod server;
pub mod settings;
pub mod system;
pub mod testing;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::mcp::ConnectionManager;
use crate::mcp::input::InputBroker;
use crate::mcp::oauth::OAuthService;
use crate::storage::Database;

/// 应用级共享状态（由 lib.rs setup 注册）
pub struct AppState {
    pub db: Database,
    pub manager: Arc<ConnectionManager>,
    /// 交互输入挂起表（v0.6.0）
    pub input: Arc<InputBroker>,
    /// OAuth 登录/凭据服务（v0.3.0）
    pub oauth: Arc<OAuthService>,
    /// 数据目录（results/ 落盘、导出默认路径）
    pub data_dir: PathBuf,
    /// 日志目录（便携模式下位于数据目录下）
    pub log_dir: PathBuf,
    /// 是否处于便携模式（exe 旁存在 mcptester.portable 且数据目录可写）
    pub portable: bool,
    /// 数据目录来源：env / portable / appdata / fallback
    pub data_dir_source: String,
    /// 当前活动测试运行（全局单运行锁；M3 使用）
    pub test_run: Arc<tokio::sync::Mutex<Option<crate::testing::RunControl>>>,
    /// 当前活动压测运行（v0.7.0）
    pub load_run: Arc<tokio::sync::Mutex<Option<Arc<crate::loadtest::LoadRunControl>>>>,
    /// 功能测试/压测统一的启动预约锁（v0.7.0）
    pub run_slot: Arc<Mutex<Option<RunSlot>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Test,
    Load,
}

impl RunKind {
    fn as_str(self) -> &'static str {
        match self {
            RunKind::Test => "测试",
            RunKind::Load => "压测",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunSlot {
    pub kind: RunKind,
    pub run_id: Option<String>,
}

/// 启动预约锁：检查 + 占位在同一把 std Mutex 内完成。
pub struct RunSlotGuard {
    slot: Arc<Mutex<Option<RunSlot>>>,
    kind: RunKind,
    committed: bool,
}

impl RunSlotGuard {
    pub fn reserve(slot: Arc<Mutex<Option<RunSlot>>>, kind: RunKind) -> Result<Self, String> {
        let mut guard = slot.lock().map_err(|_| "运行预约锁异常".to_string())?;
        if let Some(existing) = guard.as_ref() {
            return Err(format!(
                "已有{}正在运行，请先{}",
                existing.kind.as_str(),
                if existing.kind == RunKind::Load {
                    "停止"
                } else {
                    "取消"
                }
            ));
        }
        *guard = Some(RunSlot {
            kind,
            run_id: None,
        });
        Ok(Self {
            slot: Arc::clone(&slot),
            kind,
            committed: false,
        })
    }

    pub fn commit(&mut self, run_id: String) {
        if let Ok(mut guard) = self.slot.lock()
            && let Some(slot) = guard.as_mut()
            && slot.kind == self.kind
        {
            slot.run_id = Some(run_id);
            self.committed = true;
        }
    }

    pub fn release(self) {
        self.clear();
    }

    fn clear(&self) {
        if let Ok(mut guard) = self.slot.lock()
            && guard
                .as_ref()
                .is_some_and(|slot| slot.kind == self.kind)
        {
            *guard = None;
        }
    }
}

impl Drop for RunSlotGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.clear();
        }
    }
}

/// 运行结束时清预约锁（幂等）。
pub fn clear_run_slot(slot: &Arc<Mutex<Option<RunSlot>>>, kind: RunKind, run_id: &str) {
    if let Ok(mut guard) = slot.lock()
        && guard.as_ref().is_some_and(|item| {
            item.kind == kind && item.run_id.as_deref() == Some(run_id)
        })
    {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_slot_is_exclusive_and_releases() {
        let slot = Arc::new(Mutex::new(None));
        let first = RunSlotGuard::reserve(slot.clone(), RunKind::Load).expect("预约失败");
        assert!(RunSlotGuard::reserve(slot.clone(), RunKind::Test).is_err());
        drop(first);
        let mut second = RunSlotGuard::reserve(slot.clone(), RunKind::Test).expect("预约失败");
        second.commit("run-1".to_string());
        drop(second);
        assert!(slot.lock().unwrap().is_some(), "commit 后运行中应保留占位");
        clear_run_slot(&slot, RunKind::Test, "run-1");
        assert!(slot.lock().unwrap().is_none());
        assert!(RunSlotGuard::reserve(slot, RunKind::Load).is_ok());
    }
}
