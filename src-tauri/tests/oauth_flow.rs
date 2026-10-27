//! v0.3.0 OAuth 登录集成测试（需要 `mock-server` 特性）
//!
//! 运行：`cargo test --features mock-server --test oauth_flow`

#![cfg(feature = "mock-server")]

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mcptester_lib::mcp::connection::ConnectionHandle;
use mcptester_lib::mcp::error::McpError;
use mcptester_lib::mcp::models::{
    ConnectionState, HttpConfig, OAuthConfig, OAuthParam, RuntimeSettings,
};
use mcptester_lib::mcp::oauth::{OAuthService, OAuthStoreMode};
use mcptester_lib::storage::Database;
use mcptester_lib::storage::oauth_repo::RegistrationKind;
use serial_test::serial;

const MOCK_BIN: &str = env!("CARGO_BIN_EXE_mock-mcp-server");

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("端口绑定失败");
    listener.local_addr().expect("地址失败").port()
}

struct MockProcess(Child);

impl Drop for MockProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_mock(port: u16, extra: &[&str]) -> MockProcess {
    let mut command = Command::new(MOCK_BIN);
    command
        .args(["--transport", "http", "--port", &port.to_string()])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    MockProcess(command.spawn().expect("mock 服务启动失败"))
}

async fn wait_ready(port: u16) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mock 服务未在端口 {port} 就绪");
}

fn temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let db = Database::open(&dir.path().join("test.db")).expect("数据库打开失败");
    db.migrate().expect("迁移失败");
    db.with_write_blocking(|conn| {
        conn.execute(
            "INSERT INTO servers (id, name, transport, config_json, created_at, updated_at)
             VALUES ('s1', 'oauth-demo', 'streamable_http', '{}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )?;
        Ok(())
    })
    .expect("服务器插入失败");
    (dir, db)
}

fn oauth_http(port: u16, redirect_port: u16) -> HttpConfig {
    HttpConfig {
        url: format!("http://127.0.0.1:{port}/mcp"),
        headers: Vec::new(),
        timeout_secs: Some(10),
        allow_insecure_tls: false,
        oauth: Some(OAuthConfig {
            scopes: vec!["read".to_string()],
            redirect_uri: Some(format!("http://127.0.0.1:{redirect_port}/oauth/callback")),
            ..Default::default()
        }),
    }
}

async fn complete_login(service: &OAuthService, http: &HttpConfig, server_id: &str) {
    let mut events = service.subscribe();
    let start = service
        .begin_login(server_id, http, None)
        .await
        .expect("开始登录失败");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("HTTP 客户端创建失败");
    let response = client
        .get(&start.auth_url)
        .send()
        .await
        .expect("打开授权页失败");
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("授权页未返回 Location")
        .to_string();
    client.get(location).send().await.expect("回调失败");
    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("等待登录事件超时")
        .expect("登录事件通道关闭");
    assert_eq!(event.state, "authorized", "登录失败：{event:?}");
}

async fn connect(
    service: &OAuthService,
    http: &HttpConfig,
    server_id: &str,
) -> Arc<ConnectionHandle> {
    let manager = service
        .manager_for(server_id, http, OAuthStoreMode::Persistent)
        .await
        .expect("创建 manager 失败");
    let handle = ConnectionHandle::new(
        server_id,
        server_id,
        None,
        mcptester_lib::mcp::models::TransportKind::StreamableHttp,
        RuntimeSettings::default(),
    );
    handle
        .connect_http(http, Some(manager))
        .await
        .expect("OAuth 连接失败");
    handle
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_login_refresh_and_revoke() {
    keyring_core::unset_default_store();
    keyring_core::set_default_store(keyring_core::mock::Store::new().expect("mock store 创建失败"));
    let port = free_port();
    let redirect_port = free_port();
    let _mock = spawn_mock(port, &["--oauth", "--oauth-token-ttl", "1"]);
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db.clone(), false);
    let http = oauth_http(port, redirect_port);
    complete_login(&service, &http, "s1").await;

    let handle = connect(&service, &http, "s1").await;
    assert!(matches!(
        handle.state().await,
        ConnectionState::Ready { .. }
    ));

    // token 过期后自动 refresh
    tokio::time::sleep(Duration::from_secs(2)).await;
    let probe = handle.probe().await.expect("探测失败");
    assert!(probe.ok, "refresh 后探测应成功：{}", probe.detail);

    // 撤销全部 token：下一次调用应回到 oauth_required
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/test/revoke"))
        .send()
        .await
        .expect("撤销失败");
    let record = handle
        .call_tool("echo", serde_json::json!({ "text": "after-revoke" }))
        .await;
    assert_eq!(record.status, mcptester_lib::mcp::models::CallStatus::Error);
    let state = handle.state().await;
    assert!(
        matches!(
            state,
            ConnectionState::Error {
                error: McpError::OAuthRequired { .. }
            }
        ),
        "state={state:?}, error={:?}",
        record.error
    );
    handle.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_without_refresh_token_needs_relogin_after_restart() {
    keyring_core::unset_default_store();
    keyring_core::set_default_store(keyring_core::mock::Store::new().expect("mock store 创建失败"));
    let port = free_port();
    let redirect_port = free_port();
    let _mock = spawn_mock(
        port,
        &["--oauth", "--oauth-no-refresh", "--oauth-token-ttl", "60"],
    );
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db.clone(), false);
    let http = oauth_http(port, redirect_port);
    complete_login(&service, &http, "s1").await;

    // 本次会话：token cache 让首次连接成功
    let handle = connect(&service, &http, "s1").await;
    assert!(matches!(
        handle.state().await,
        ConnectionState::Ready { .. }
    ));
    handle.disconnect().await;

    // 模拟重启：新 service / cache；无 refresh token → 连接需要重新登录
    let restarted = OAuthService::new(db, false);
    let manager = restarted
        .manager_for("s1", &http, OAuthStoreMode::Persistent)
        .await
        .expect("重启后 manager 失败");
    let handle = ConnectionHandle::new(
        "s1",
        "s1",
        None,
        mcptester_lib::mcp::models::TransportKind::StreamableHttp,
        RuntimeSettings::default(),
    );
    let error = handle
        .connect_http(&http, Some(manager))
        .await
        .expect_err("无 refresh token 重启后应需要登录");
    assert_eq!(error.kind(), "oauth_required");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_dcr_secret_survives_restart() {
    keyring_core::unset_default_store();
    keyring_core::set_default_store(keyring_core::mock::Store::new().expect("mock store 创建失败"));
    let port = free_port();
    let redirect_port = free_port();
    let _mock = spawn_mock(
        port,
        &["--oauth", "--oauth-dcr-secret", "--oauth-token-ttl", "1"],
    );
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db.clone(), false);
    let http = oauth_http(port, redirect_port);
    complete_login(&service, &http, "s1").await;
    let handle = connect(&service, &http, "s1").await;
    assert!(matches!(
        handle.state().await,
        ConnectionState::Ready { .. }
    ));
    handle.disconnect().await;

    // 重启后仍需带 DCR secret 才能 refresh
    let restarted = OAuthService::new(db, false);
    let handle = connect(&restarted, &http, "s1").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let probe = handle.probe().await.expect("探测失败");
    assert!(
        probe.ok,
        "DCR secret 重启后 refresh 应成功：{}",
        probe.detail
    );
    handle.disconnect().await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_authorize_params_are_forwarded() {
    let port = free_port();
    let _mock = spawn_mock(
        port,
        &[
            "--oauth",
            "--oauth-require-authorize-param",
            "audience=https://api.example.com",
        ],
    );
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db, true);
    let mut http = oauth_http(port, free_port());
    http.oauth.as_mut().expect("oauth").authorize_params = vec![OAuthParam {
        key: "audience".to_string(),
        value: "https://api.example.com".to_string(),
    }];
    complete_login(&service, &http, "s1").await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_missing_authorize_param_fails() {
    let port = free_port();
    let _mock = spawn_mock(
        port,
        &[
            "--oauth",
            "--oauth-require-authorize-param",
            "audience=https://api.example.com",
        ],
    );
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db, true);
    let http = oauth_http(port, free_port());
    let start = service
        .begin_login("s1", &http, None)
        .await
        .expect("开始登录失败");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("HTTP 客户端创建失败");
    let response = client
        .get(&start.auth_url)
        .send()
        .await
        .expect("打开授权页失败");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    service
        .cancel_login(&start.flow_id)
        .await
        .expect("取消失败");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_diagnostics_refresh_and_access_token() {
    let port = free_port();
    let _mock = spawn_mock(port, &["--oauth"]);
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db, true);
    let http = oauth_http(port, free_port());
    complete_login(&service, &http, "s1").await;

    let diagnostics = service
        .diagnostics("s1", &http, None)
        .await
        .expect("诊断失败");
    assert!(diagnostics.issuer.as_deref().is_some_and(|value| !value.is_empty()));
    assert!(diagnostics.authorization_endpoint.is_some());
    assert!(diagnostics.metadata.is_some());
    assert_eq!(diagnostics.registration, Some(RegistrationKind::Dcr));

    let status = service
        .force_refresh("s1", &http, None)
        .await
        .expect("强制刷新失败");
    assert!(status.authorized);

    let token = service
        .access_token("s1", &http, None)
        .await
        .expect("读取 access token 失败");
    assert!(!token.access_token.is_empty());
    assert!(token.token_type.eq_ignore_ascii_case("bearer"));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_logout_revokes_refresh_token() {
    let port = free_port();
    let _mock = spawn_mock(port, &["--oauth"]);
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db.clone(), true);
    let http = oauth_http(port, free_port());
    complete_login(&service, &http, "s1").await;

    let result = service
        .logout("s1", Some(&http), None)
        .await
        .expect("退出失败");
    assert!(result.revoked, "应调用 revocation_endpoint：{result:?}");

    let row = db
        .with_read_blocking(|conn| mcptester_lib::storage::oauth_repo::get(conn, "s1"))
        .expect("查询失败")
        .expect("行不存在");
    assert!(!row.has_token);
    assert!(!row.has_refresh_token);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn oauth_cimd_login_survives_manager_for() {
    let port = free_port();
    let _mock = spawn_mock(port, &["--oauth", "--oauth-cimd"]);
    wait_ready(port).await;

    let (_dir, db) = temp_db();
    let service = OAuthService::new(db.clone(), true);
    let mut http = oauth_http(port, free_port());
    http.oauth.as_mut().expect("oauth").client_metadata_url =
        Some("https://client.example.com/oauth-client-metadata.json".to_string());

    complete_login(&service, &http, "s1").await;

    // 登录后立刻 manager_for：CIMD 指纹必须与保存时一致，不能清 token
    let handle = connect(&service, &http, "s1").await;
    assert!(matches!(
        handle.state().await,
        ConnectionState::Ready { .. }
    ));

    let row = db
        .with_read_blocking(|conn| mcptester_lib::storage::oauth_repo::get(conn, "s1"))
        .expect("查询失败")
        .expect("行不存在");
    assert_eq!(row.registration_kind, Some(RegistrationKind::Cimd));
    assert!(row.client_secret.is_none());
    assert!(row.client_secret_ref.is_none());
    handle.disconnect().await;
}
