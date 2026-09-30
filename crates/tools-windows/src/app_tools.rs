//! Application launch tool.
//!
//! Resolves the executable search order:
//! 1. Absolute path (`C:\...` / `C:/...`) — used as-is.
//! 2. Bare name with extension (`notepad.exe`) — searched on the working dir
//!    then the system PATH.
//! 3. Bare name without extension (`notepad`) — same as above with `.exe`
//!    appended.
//!
//! Because we cannot reliably expand `PATH` at spawn time without invoking a
//! shell, we deliberately route bare names through `cmd /C start` so the
//! shell's own PATH lookup applies.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::path::Path;
use tokio::process::Command;

/// Launch an application by path or name.
pub struct OpenAppTool;

#[async_trait]
impl Tool for OpenAppTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "open_app".into(),
            description: "启动一个应用程序。可以传入可执行文件的绝对路径，或程序名（如 notepad、explorer）。程序会在后台启动，不会阻塞。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name_or_path": {"type": "string", "description": "可执行文件路径或程序名"}
                },
                "required": ["name_or_path"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Dangerous
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let input = match params.get("name_or_path").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return ToolResult::err("缺少有效的 name_or_path 参数"),
        };

        match launch(input).await {
            Ok(description) => ToolResult::ok(format!("已启动: {description}")),
            Err(e) => ToolResult::err(format!("启动失败: {e}")),
        }
    }
}

/// Launch `input`, returning a short description of what was started.
async fn launch(input: &str) -> Result<String, String> {
    let path = Path::new(input);

    // 1. Absolute path: spawn directly.
    if path.is_absolute() {
        if !path.exists() {
            return Err(format!("路径不存在: {input}"));
        }
        Command::new(path)
            .spawn()
            .map_err(|e| format!("无法启动 {input}: {e}"))?;
        return Ok(input.to_string());
    }

    // 2/3. Bare name: let cmd.exe resolve it via PATH / App Paths.
    // `start ""` with an empty title prevents the first quoted argument from
    // being treated as the console window title.
    let with_ext = if input.to_lowercase().ends_with(".exe") {
        input.to_string()
    } else {
        format!("{input}.exe")
    };

    // Try both the original name and the .exe-suffixed form. cmd's `start`
    // will use the shell's PATH lookup plus the App Paths registry key, which
    // is how users expect `open_app("chrome")` to work.
    for candidate in [&with_ext, input] {
        let status = Command::new("cmd")
            .args(["/C", "start", "", candidate])
            .spawn()
            .map_err(|e| format!("无法启动 cmd: {e}"))?
            .wait()
            .await
            .map_err(|e| format!("启动失败: {e}"))?;

        if status.success() {
            return Ok(candidate.to_string());
        }
    }

    Err(format!(
        "找不到程序 \"{input}\"。请提供可执行文件的完整路径，或确认程序名正确且已安装。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_missing_absolute_path() {
        let result = launch("C:\\nonexistent\\missing.exe").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("路径不存在"));
    }
}
