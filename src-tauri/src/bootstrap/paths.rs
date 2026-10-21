//! 运行目录解析：数据目录 / 日志目录 / 便携模式判定。
//!
//! 便携模式由 exe 同目录的 `mcptester.portable` 标记文件启用；
//! `MCPTESTER_DATA_DIR` 可覆盖数据目录（相对路径按 exe 目录解析）。
//! 便携数据目录不可写时回退 AppData，并关闭便携凭据策略。

use std::path::{Path, PathBuf};

/// 便携模式标记文件名
pub const PORTABLE_MARKER: &str = "mcptester.portable";

/// 数据目录来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    Env,
    Portable,
    AppData,
    /// 标记为便携但目录不可写，回退 AppData
    Fallback,
}

impl DataDirSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DataDirSource::Env => "env",
            DataDirSource::Portable => "portable",
            DataDirSource::AppData => "appdata",
            DataDirSource::Fallback => "fallback",
        }
    }
}

/// 解析后的运行目录
#[derive(Debug, Clone)]
pub struct ResolvedPaths {
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub portable: bool,
    pub source: DataDirSource,
    /// 回退等非致命问题（供设置页/日志展示）
    pub warning: Option<String>,
}

/// 解析运行目录。
///
/// 参数显式传入，便于单测；生产调用由 `lib.rs` 从 Tauri AppHandle 取值。
pub fn resolve(
    exe_dir: &Path,
    app_data_dir: &Path,
    app_local_data_dir: &Path,
    env_data_dir: Option<&Path>,
) -> anyhow::Result<ResolvedPaths> {
    let marker = exe_dir.join(PORTABLE_MARKER).is_file();

    if let Some(env_dir) = env_data_dir {
        let data_dir = if env_dir.is_absolute() {
            env_dir.to_path_buf()
        } else {
            exe_dir.join(env_dir)
        };
        std::fs::create_dir_all(&data_dir).map_err(|e| {
            anyhow::anyhow!("MCPTESTER_DATA_DIR 不可用（{}）：{e}", data_dir.display())
        })?;
        let log_dir = data_dir.join("logs");
        std::fs::create_dir_all(&log_dir)?;
        return Ok(ResolvedPaths {
            data_dir,
            log_dir,
            portable: marker,
            source: DataDirSource::Env,
            warning: None,
        });
    }

    if marker {
        let data_dir = exe_dir.join("data");
        if writable_dir(&data_dir) {
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            return Ok(ResolvedPaths {
                data_dir,
                log_dir,
                portable: true,
                source: DataDirSource::Portable,
                warning: None,
            });
        }

        let log_dir = app_local_data_dir.join("logs");
        std::fs::create_dir_all(&log_dir)?;
        return Ok(ResolvedPaths {
            data_dir: app_data_dir.to_path_buf(),
            log_dir,
            portable: false,
            source: DataDirSource::Fallback,
            warning: Some(format!(
                "便携数据目录不可写（{}），已回退到 {}；本次运行不启用便携凭据策略",
                data_dir.display(),
                app_data_dir.display()
            )),
        });
    }

    std::fs::create_dir_all(app_data_dir)?;
    let log_dir = app_local_data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;
    Ok(ResolvedPaths {
        data_dir: app_data_dir.to_path_buf(),
        log_dir,
        portable: false,
        source: DataDirSource::AppData,
        warning: None,
    })
}

/// 创建目录并做一次写入探测（create_dir_all 对已存在但只读的目录会成功）
fn writable_dir(path: &Path) -> bool {
    if std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".mcptester-write-test");
    let result = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
        .map(|_| ())
        .and_then(|_| std::fs::remove_file(&probe));
    result.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let root = tempfile::tempdir().expect("临时目录创建失败");
        let exe = root.path().join("app");
        let app_data = root.path().join("appdata");
        let local = root.path().join("local");
        std::fs::create_dir_all(&exe).unwrap();
        (root, exe, app_data, local)
    }

    #[test]
    fn appdata_by_default() {
        let (_root, exe, app_data, local) = dirs();
        let paths = resolve(&exe, &app_data, &local, None).expect("解析失败");
        assert_eq!(paths.data_dir, app_data);
        assert_eq!(paths.source, DataDirSource::AppData);
        assert!(!paths.portable);
        assert_eq!(paths.log_dir, local.join("logs"));
    }

    #[test]
    fn portable_marker_uses_exe_data_dir() {
        let (_root, exe, app_data, local) = dirs();
        std::fs::write(exe.join(PORTABLE_MARKER), b"").unwrap();
        let paths = resolve(&exe, &app_data, &local, None).expect("解析失败");
        assert_eq!(paths.data_dir, exe.join("data"));
        assert_eq!(paths.source, DataDirSource::Portable);
        assert!(paths.portable);
        assert_eq!(paths.log_dir, exe.join("data").join("logs"));
    }

    #[test]
    fn env_data_dir_overrides_marker_location() {
        let (root, exe, app_data, local) = dirs();
        std::fs::write(exe.join(PORTABLE_MARKER), b"").unwrap();
        let custom = root.path().join("custom-data");
        let paths = resolve(&exe, &app_data, &local, Some(&custom)).expect("解析失败");
        assert_eq!(paths.data_dir, custom);
        assert_eq!(paths.source, DataDirSource::Env);
        assert!(paths.portable); // marker 决定凭据策略
        assert_eq!(paths.log_dir, custom.join("logs"));
    }

    #[test]
    fn relative_env_data_dir_is_relative_to_exe() {
        let (_root, exe, app_data, local) = dirs();
        let paths =
            resolve(&exe, &app_data, &local, Some(Path::new("profile-a"))).expect("解析失败");
        assert_eq!(paths.data_dir, exe.join("profile-a"));
    }
}
