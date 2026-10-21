//! stdio 传输：命令解析（`resolve` / `cmd_shell` / `never`）+ 进程树回收 + stderr 捕获
//!
//! 事实依据（复审确认）：Windows 上带全路径的 `.cmd` 可直接启动，真正的坑是
//! 裸命令名的 PATH/PATHEXT 解析；rmcp `which_command` 专门解决该问题（默认 `resolve`）。

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use process_wrap::tokio::CommandWrap;
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use rmcp::transport::child_process::TokioChildProcess;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::mcp::error::McpError;
use crate::mcp::handler::EventHub;
use crate::mcp::models::{ShellWrap, StdioConfig};

/// 构建 stdio 传输：spawn 子进程并启动 stderr 读取任务（进程树回收已绑定）
pub fn build_stdio(
    cfg: &StdioConfig,
    hub: &Arc<EventHub>,
) -> Result<TokioChildProcess, McpError> {
    let (program, args) = resolve_program(cfg)?;

    let envs: HashMap<String, String> = cfg
        .env
        .iter()
        .filter_map(|kv| kv.value.as_ref().map(|v| (kv.key.clone(), v.clone())))
        .collect();
    let cwd = cfg.cwd.clone();

    let mut wrap = CommandWrap::with_new(&program, |cmd| {
        cmd.args(&args);
        if !envs.is_empty() {
            cmd.envs(&envs);
        }
        if let Some(dir) = &cwd {
            cmd.current_dir(dir);
        }
    });

    // 显式绑定：进程树随父进程退出而回收（Windows Job / Unix ProcessGroup）
    #[cfg(windows)]
    wrap.wrap(JobObject);
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::leader());

    let (transport, stderr) = TokioChildProcess::builder(wrap)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(cfg, e))?;

    match stderr {
        Some(stderr) => {
            let hub = Arc::clone(hub);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => hub.push_stderr(line),
                        Ok(None) => break,
                        Err(e) => {
                            hub.push_stderr(format!("[mcp] 读取 stderr 失败：{e}"));
                            break;
                        }
                    }
                }
            });
        }
        None => hub.push_stderr("[mcp] stderr 未捕获（管道创建失败）"),
    }

    Ok(transport)
}

/// 按 `shell_wrap` 模式解析可执行文件（程序与参数）
fn resolve_program(cfg: &StdioConfig) -> Result<(String, Vec<String>), McpError> {
    match cfg.shell_wrap {
        ShellWrap::Resolve => {
            let resolved =
                rmcp::transport::which_command(&cfg.command).map_err(|e| McpError::SpawnFailed {
                    message: format!(
                        "找不到命令「{}」：{e}（可在高级设置改为 cmd_shell 或检查 PATH）",
                        cfg.command
                    ),
                })?;
            let program = resolved
                .as_std()
                .get_program()
                .to_string_lossy()
                .to_string();
            Ok((program, cfg.args.clone()))
        }
        ShellWrap::CmdShell => {
            #[cfg(windows)]
            {
                let mut args = vec!["/C".to_string(), cfg.command.clone()];
                args.extend(cfg.args.clone());
                Ok(("cmd".to_string(), args))
            }
            #[cfg(unix)]
            {
                let joined = format!("{} {}", cfg.command, cfg.args.join(" "));
                Ok(("sh".to_string(), vec!["-c".to_string(), joined]))
            }
        }
        ShellWrap::Never => Ok((cfg.command.clone(), cfg.args.clone())),
    }
}

fn spawn_error(cfg: &StdioConfig, e: std::io::Error) -> McpError {
    let hint = match cfg.shell_wrap {
        ShellWrap::Resolve => {
            "（提示：命令需在 PATH 中；也可在高级设置改为 cmd_shell）"
        }
        ShellWrap::CmdShell => "（提示：cmd_shell 存在参数转义风险，可尝试 resolve）",
        ShellWrap::Never => "（提示：never 模式不做路径解析，请填完整路径）",
    };
    McpError::SpawnFailed {
        message: format!("启动进程失败：{e}{hint}"),
    }
}
