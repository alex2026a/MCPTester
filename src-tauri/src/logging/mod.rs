//! 日志模块：文件日志初始化与字段级脱敏（见设计文档 03 第 7.3 节）
//!
//! 原则：
//! - 日志按天滚动输出到本地日志目录，非阻塞写入；
//! - 脱敏 = 精确字段名集合（非子串匹配，避免 monkey/keyboard 误伤）+ 值模式打码 + 200 字符截断；
//! - stderr 与服务器通知只进内存环形缓冲，不写日志文件（由 mcp 模块保证）。

// M1 脚手架：脱敏工具在 M2（网络/传输日志接入）后被调用，届时移除此 allow
#![allow(dead_code)]

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use tracing_subscriber::EnvFilter;

/// 日志守卫：持有它以保证非阻塞日志线程存活（由 Tauri 状态管理持有）
pub struct LogGuard {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

/// 初始化日志（进程生命周期内仅调用一次）
pub fn init(log_dir: &Path) -> anyhow::Result<LogGuard> {
    let appender = tracing_appender::rolling::daily(log_dir, "mcptester.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .init();

    Ok(LogGuard { _guard: guard })
}

/// 敏感字段名精确集合（小写比较；同时匹配去 `x-` 前缀后的名字）
const SENSITIVE_EXACT: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "token",
    "access_token",
    "api-key",
    "apikey",
    "key",
    "secret",
    "password",
    "credential",
];

/// 敏感字段名后缀（归一化后匹配，如 `github-token`、`client-secret`）
const SENSITIVE_SUFFIX: &[&str] = &["-key", "-token", "-secret", "-password", "-credential"];

/// 敏感字段名判定：精确匹配（含 `x-` 前缀剥离与后缀规则），不做任意子串匹配
pub fn is_sensitive_field(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    let normalized = lowered.strip_prefix("x-").unwrap_or(&lowered);
    if SENSITIVE_EXACT.contains(&normalized) {
        return true;
    }
    SENSITIVE_SUFFIX.iter().any(|suffix| normalized.ends_with(suffix))
}

/// 值模式：Bearer 令牌
static RE_BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(bearer\s+)[A-Za-z0-9\-._~+/=]{8,}").expect("Bearer 正则编译失败")
});

/// 值模式：JWT 形态（三段 base64url）
static RE_JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}")
        .expect("JWT 正则编译失败")
});

/// 值模式：sk- 前缀密钥
static RE_SK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{8,}").expect("sk- 正则编译失败"));

/// 值模式：URL query 中的 token/key/secret 参数
static RE_URL_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([?&](?:access[_-]?token|token|api[_-]?key|apikey|key|secret|password)=)[^&\s"']+"#)
        .expect("URL token 正则编译失败")
});

/// 值模式：URL userinfo（`scheme://user:pass@host`）
static RE_URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(://)[^/\s:@]+:[^/\s@]+@").expect("URL userinfo 正则编译失败")
});

/// 文本脱敏：应用值模式打码后截断至 200 字符
pub fn scrub_text(value: &str) -> String {
    let mut s = RE_BEARER.replace_all(value, "${1}[REDACTED]").into_owned();
    s = RE_JWT.replace_all(&s, "[REDACTED:JWT]").into_owned();
    s = RE_SK.replace_all(&s, "[REDACTED:KEY]").into_owned();
    s = RE_URL_USERINFO.replace_all(&s, "${1}[REDACTED]@").into_owned();
    s = RE_URL_TOKEN.replace_all(&s, "${1}[REDACTED]").into_owned();
    truncate(&s, 200)
}

/// 按字段名脱敏：敏感字段全量打码，其余走值模式打码 + 截断
pub fn redacted_value(field: &str, value: &str) -> String {
    if is_sensitive_field(field) {
        "[REDACTED]".to_string()
    } else {
        scrub_text(value)
    }
}

/// 截断字符串到最大字符数（超出部分以省略号结尾）
pub fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut s: String = value.chars().take(max_chars).collect();
        s.push('…');
        s
    }
}

/// 脱敏展示包装器：日志中输出敏感值时使用，如 `tracing::debug!(key = %Redacted(&key), ...)`
pub struct Redacted<'a>(pub &'a str);

impl std::fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED:{}chars]", self.0.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_field_name_matching() {
        // 字段名判定（表驱动：命中 / 误伤 / 前缀与后缀规则）
        let cases: &[(&str, bool)] = &[
            // 命中
            ("authorization", true),
            ("Authorization", true),
            ("cookie", true),
            ("set-cookie", true),
            ("token", true),
            ("api-key", true),
            ("api_key", false), // 下划线形式不在精确集合中（由调用方归一或值模式兜底）
            ("apikey", true),
            ("key", true),
            ("password", true),
            ("credential", true),
            ("X-Api-Key", true),
            ("x-auth-token", true),   // 去 x- 前缀 → auth-token，命中 -token 后缀
            ("github-token", true),
            ("client-secret", true),
            // 误伤检查（子串不匹配）
            ("monkey", false),
            ("keyboard", false),
            ("keys", false),
            ("tokens_count", false),
        ];
        for (name, expected) in cases {
            assert_eq!(is_sensitive_field(name), *expected, "字段 {name} 判定不符预期");
        }
    }

    #[test]
    fn redacted_value_masks_sensitive_fields() {
        assert_eq!(redacted_value("Authorization", "Bearer abc"), "[REDACTED]");
        assert_eq!(redacted_value("X-Api-Key", "abc"), "[REDACTED]");
    }

    #[test]
    fn scrub_text_masks_value_patterns() {
        // 漏检兜底：非敏感字段名中的敏感值形态
        assert_eq!(
            scrub_text("Authorization: Bearer abcdefgh12345678"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            scrub_text("key=sk-abcdefgh12345678"),
            "key=[REDACTED:KEY]"
        );
        assert!(
            !scrub_text("token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcd")
                .contains("eyJzdWIi")
        );
        assert_eq!(
            scrub_text("访问 https://example.com/mcp?token=abc123&x=1 失败"),
            "访问 https://example.com/mcp?token=[REDACTED]&x=1 失败"
        );
        assert_eq!(
            scrub_text("https://user:pass123@example.com/mcp"),
            "https://[REDACTED]@example.com/mcp"
        );
    }

    #[test]
    fn scrub_text_does_not_false_positive() {
        // 误伤检查：普通文本不被改动
        assert_eq!(scrub_text("monkey keyboard"), "monkey keyboard");
        assert_eq!(scrub_text("keys are tokens_count"), "keys are tokens_count");
    }

    #[test]
    fn redacted_value_truncates_others() {
        // 非敏感字段截断到 200 字符
        let long = "a".repeat(250);
        let out = redacted_value("message", &long);
        assert_eq!(out.chars().count(), 201); // 200 字符 + 省略号
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_short_value_unchanged() {
        assert_eq!(truncate("短文本", 200), "短文本");
    }

    #[test]
    fn redacted_display_hides_content() {
        // Redacted 显示不包含原文
        let s = format!("{}", Redacted("sk-secret-value"));
        assert!(!s.contains("secret"));
        assert!(s.contains("REDACTED"));
    }
}
