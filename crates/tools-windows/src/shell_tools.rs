//! Shell command execution tool.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::time::Duration;
use tokio::process::Command;

/// Default timeout for a shell command.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Maximum allowed timeout (cap so the agent cannot hang indefinitely).
const MAX_TIMEOUT_SECS: u64 = 120;
/// Maximum output bytes before truncation, matching `read_file`'s limit.
const MAX_OUTPUT_BYTES: usize = 50_000;

/// Execute a shell command and return its stdout/stderr.
pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "run_command".into(),
            description: "执行系统命令并返回输出。此工具具有风险，每次执行前需要用户确认。命令最长运行 30 秒（可通过 timeout_seconds 调整，上限 120 秒），超时会被终止。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "要执行的命令"},
                    "cwd": {"type": "string", "description": "工作目录（可选）"},
                    "timeout_seconds": {"type": "integer", "description": "超时时间（秒），默认 30，最大 120"}
                },
                "required": ["command"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Dangerous
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("缺少 command 参数"),
        };

        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let timeout_secs = params
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);
        let timeout = Duration::from_secs(timeout_secs);

        // On Windows, cmd.exe defaults to the system OEM code page (cp936/GBK
        // on Chinese Windows). Prepend `chcp 65001` so subsequent output is
        // UTF-8 and `String::from_utf8_lossy` decodes correctly instead of
        // producing mojibake.
        let (program, args): (&str, Vec<String>) = if cfg!(target_os = "windows") {
            let wrapped = format!("chcp 65001 >nul 2>&1 & {command}");
            ("cmd", vec!["/C".into(), wrapped])
        } else {
            ("sh", vec!["-c".into(), command.into()])
        };

        // Use tokio::process::Command (not std) so we can:
        // 1. Race the child against a timeout via tokio::time::timeout.
        // 2. Kill the child on drop (kill_on_drop) when the timeout fires.
        // 3. Redirect stdin to null so a command that reads stdin does not
        //    hang forever waiting for input that will never arrive.
        let mut cmd = Command::new(program);
        cmd.args(&args)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => return ToolResult::err(format!("启动命令失败: {e}")),
        };

        // `wait_with_output` consumes the child; when the timeout future is
        // dropped the child (inside it) is dropped too, and kill_on_drop
        // terminates the OS process so it cannot outlive the tool call.
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                if exit_code == 0 {
                    let text = if stdout.is_empty() && stderr.is_empty() {
                        "命令执行成功（无输出）".to_string()
                    } else if stderr.is_empty() {
                        truncate_output(&stdout)
                    } else {
                        truncate_output(&format!("stdout:\n{stdout}\nstderr:\n{stderr}"))
                    };
                    ToolResult::ok(text)
                } else {
                    ToolResult::err(truncate_output(&format!(
                        "命令退出码 {exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}"
                    )))
                }
            }
            Ok(Err(e)) => ToolResult::err(format!("命令执行失败: {e}")),
            Err(_) => ToolResult::err(format!(
                "命令在 {timeout_secs} 秒后超时被终止。如需更长运行时间，请指定 timeout_seconds 参数（上限 {MAX_TIMEOUT_SECS} 秒）。"
            )),
        }
    }
}

/// Truncate output at a valid UTF-8 boundary to avoid overwhelming the LLM
/// context or the IPC channel.
fn truncate_output(content: &str) -> String {
    if content.len() <= MAX_OUTPUT_BYTES {
        return content.to_string();
    }
    let mut end = MAX_OUTPUT_BYTES.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n...(输出已截断，共 {} 字节)",
        &content[..end],
        content.len()
    )
}
