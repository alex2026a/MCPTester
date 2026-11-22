//! 压测临时会话工厂：独立、静默、可取消。

use std::sync::Arc;

use crate::mcp::connection::ConnectionHandle;
use crate::mcp::models::{
    HttpConfig, RuntimeSettings, ServerConfig, StdioConfig, TransportKind,
};
use crate::mcp::oauth::{OAuthService, OAuthStoreMode};

use super::models::LoadProfile;

pub const LOAD_CONNECT_TIMEOUT_SECS: u64 = 10;

/// 每个 VU 一个独立句柄；不注册 `ConnectionManager`，不推 UI 事件。
pub struct LoadSessionFactory {
    config: ServerConfig,
    settings: RuntimeSettings,
    oauth: Option<Arc<OAuthService>>,
}

impl LoadSessionFactory {
    pub fn new(
        config: ServerConfig,
        profile: &LoadProfile,
        oauth: Option<Arc<OAuthService>>,
    ) -> Self {
        let settings = RuntimeSettings {
            request_timeout_secs: profile.timeout_secs.max(1),
            connect_timeout_secs: LOAD_CONNECT_TIMEOUT_SECS,
            stderr_buffer_lines: 8,
            notification_buffer_lines: 8,
            wire_buffer_lines: 10,
            wire_enabled: false,
            capture_enabled: false,
        };
        Self {
            config,
            settings,
            oauth,
        }
    }

    pub fn server_id(&self) -> &str {
        &self.config.id
    }

    pub fn server_name(&self) -> &str {
        &self.config.name
    }

    pub fn settings(&self) -> &RuntimeSettings {
        &self.settings
    }

    /// 建立第 `index` 个会话；调用方负责取消/超时与最终清理。
    pub async fn connect(&self, index: u32) -> Result<Arc<ConnectionHandle>, String> {
        let handle = ConnectionHandle::new(
            format!("{}#lt-{index}", self.config.id),
            format!("{}（压测）", self.config.name),
            self.config.protocol_version_pin.clone(),
            self.config.transport.clone(),
            self.settings.clone(),
        );
        let result = match self.config.transport {
            TransportKind::Stdio => {
                let cfg: &StdioConfig = self
                    .config
                    .stdio
                    .as_ref()
                    .ok_or_else(|| "缺少 stdio 配置".to_string())?;
                handle.connect_stdio(cfg).await.map_err(|e| e.message())
            }
            TransportKind::StreamableHttp => {
                let cfg: &HttpConfig = self
                    .config
                    .http
                    .as_ref()
                    .ok_or_else(|| "缺少 HTTP 配置".to_string())?;
                let auth_manager = if cfg.oauth.is_some() {
                    let oauth = self
                        .oauth
                        .as_ref()
                        .ok_or_else(|| "OAuth 连接缺少认证服务".to_string())?;
                    Some(
                        oauth
                            .manager_for(&self.config.id, cfg, OAuthStoreMode::Persistent)
                            .await
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    None
                };
                handle
                    .connect_http(cfg, auth_manager)
                    .await
                    .map_err(|e| e.message())
            }
            TransportKind::Sse => {
                let cfg: &HttpConfig = self
                    .config
                    .http
                    .as_ref()
                    .ok_or_else(|| "缺少 HTTP 配置".to_string())?;
                handle.connect_sse(cfg).await.map_err(|e| e.message())
            }
        };
        match result {
            Ok(()) => Ok(handle),
            Err(error) => {
                handle.disconnect().await;
                Err(format!("压测会话 {index} 连接失败：{error}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::models::{KeyValue, ShellWrap, StdioConfig};
    use serde_json::json;

    fn config(transport: TransportKind) -> ServerConfig {
        ServerConfig {
            id: "s1".to_string(),
            name: "demo".to_string(),
            transport: transport.clone(),
            stdio: matches!(transport, TransportKind::Stdio).then(|| StdioConfig {
                command: "mock".to_string(),
                args: Vec::new(),
                env: Vec::<KeyValue>::new(),
                cwd: None,
                shell_wrap: ShellWrap::Never,
            }),
            http: None,
            protocol_version_pin: None,
            enabled: true,
            tags: Vec::new(),
            origin: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_connected_at: None,
        }
    }

    #[test]
    fn quiet_settings_disable_wire_and_capture() {
        let profile = LoadProfile {
            timeout_secs: 12,
            ..Default::default()
        };
        let factory = LoadSessionFactory::new(config(TransportKind::Stdio), &profile, None);
        let settings = factory.settings();
        assert!(!settings.wire_enabled);
        assert!(!settings.capture_enabled);
        assert_eq!(settings.request_timeout_secs, 12);
        assert_eq!(settings.connect_timeout_secs, LOAD_CONNECT_TIMEOUT_SECS);
        assert!(!json!(settings.wire_buffer_lines).is_null());
    }
}
