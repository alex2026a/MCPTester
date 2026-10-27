//! 连接管理器：多服务器并发连接的生命周期编排（见设计文档 03 第 3.1 节）

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use rmcp::transport::auth::AuthorizationManager;

use super::connection::ConnectionHandle;
use super::error::McpError;
use super::models::{RuntimeSettings, ServerConfig, TransportKind};
use super::oauth::AuthStateSink;

/// 连接管理器（应用级单例，由 AppState 持有）
pub struct ConnectionManager {
    settings: RuntimeSettings,
    connections: Mutex<HashMap<String, Arc<ConnectionHandle>>>,
    auth_state: Option<Arc<dyn AuthStateSink>>,
}

impl ConnectionManager {
    pub fn new(settings: RuntimeSettings, auth_state: Option<Arc<dyn AuthStateSink>>) -> Self {
        Self {
            settings,
            connections: Mutex::new(HashMap::new()),
            auth_state,
        }
    }

    /// 当前生效的运行时设置
    pub fn settings(&self) -> &RuntimeSettings {
        &self.settings
    }

    /// 获取已存在的连接句柄
    pub async fn handle(&self, server_id: &str) -> Option<Arc<ConnectionHandle>> {
        self.connections.lock().await.get(server_id).cloned()
    }

    /// 获取或创建连接句柄（不发起连接）
    pub async fn get_or_create(&self, config: &ServerConfig) -> Arc<ConnectionHandle> {
        let mut guard = self.connections.lock().await;
        if let Some(existing) = guard.get(&config.id) {
            return Arc::clone(existing);
        }
        let handle = ConnectionHandle::new(
            config.id.clone(),
            config.name.clone(),
            config.protocol_version_pin.clone(),
            config.transport.clone(),
            self.settings.clone(),
        );
        if let Some(sink) = &self.auth_state {
            handle.set_auth_state_sink(Arc::clone(sink)).await;
        }
        guard.insert(config.id.clone(), Arc::clone(&handle));
        handle
    }

    /// 连接（若已连接则先断开再连）
    pub async fn connect(
        &self,
        config: &ServerConfig,
        auth_manager: Option<AuthorizationManager>,
    ) -> Result<Arc<ConnectionHandle>, McpError> {
        let handle = self.get_or_create(config).await;
        if handle.state().await.is_ready() {
            handle.disconnect().await;
        }
        let result = match config.transport {
            TransportKind::Stdio => {
                let stdio = config.stdio.as_ref().ok_or_else(|| McpError::Config {
                    message: "缺少 stdio 配置".to_string(),
                })?;
                handle.connect_stdio(stdio).await
            }
            TransportKind::StreamableHttp => {
                let http = config.http.as_ref().ok_or_else(|| McpError::Config {
                    message: "缺少 HTTP 配置".to_string(),
                })?;
                handle.connect_http(http, auth_manager).await
            }
            TransportKind::Sse => {
                let http = config.http.as_ref().ok_or_else(|| McpError::Config {
                    message: "缺少 SSE 配置".to_string(),
                })?;
                handle.connect_sse(http).await
            }
        };
        result.map(|_| handle)
    }

    /// 断开（保留句柄，可重连）
    pub async fn disconnect(&self, server_id: &str) {
        if let Some(handle) = self.handle(server_id).await {
            handle.disconnect().await;
        }
    }

    /// 移除句柄并断开（删除服务器时调用）
    pub async fn remove(&self, server_id: &str) {
        let handle = self.connections.lock().await.remove(server_id);
        if let Some(handle) = handle {
            handle.disconnect().await;
        }
    }

    /// 全部连接句柄（事件转发/退出清理用）
    pub async fn all_handles(&self) -> Vec<(String, Arc<ConnectionHandle>)> {
        let guard = self.connections.lock().await;
        guard
            .iter()
            .map(|(id, handle)| (id.clone(), Arc::clone(handle)))
            .collect()
    }

    /// 退出清理：断开全部连接（应用退出流程调用）
    pub async fn shutdown_all(&self) {
        let handles: Vec<Arc<ConnectionHandle>> = {
            let mut guard = self.connections.lock().await;
            guard.drain().map(|(_, handle)| handle).collect()
        };
        for handle in handles {
            handle.disconnect().await;
        }
    }
}
