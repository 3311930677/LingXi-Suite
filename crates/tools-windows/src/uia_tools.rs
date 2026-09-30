//! UIA-based tools: read the current selection and write text to the focused control.

use assistant_core::InputAdapter;
use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;

/// Read the currently selected text in the foreground application.
pub struct ReadSelectionTool;

#[async_trait]
impl Tool for ReadSelectionTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_selection".into(),
            description: "读取当前前台应用中选中的文字。用户需要先在任意应用中选中文字。".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(&self, _params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let adapter = assistant_windows::WindowsAdapter::new();
        match adapter.capture_selection() {
            Ok(snapshot) => {
                if snapshot.selected_text.is_empty() {
                    ToolResult::err("没有选中的文字")
                } else {
                    ToolResult::ok(snapshot.selected_text)
                }
            }
            Err(e) => ToolResult::err(format!("读取选区失败: {e}")),
        }
    }
}

/// Write text to the focused control, replacing any current selection.
pub struct WriteTextTool;

#[async_trait]
impl Tool for WriteTextTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_text".into(),
            description: "向当前焦点控件写入文本（替换模式）。写入前需确保目标控件已聚焦。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "要写入的文本"}
                },
                "required": ["text"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let text = match params.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::err("缺少 text 参数"),
        };

        // Use the clipboard paste path: set clipboard, then simulate Ctrl+V.
        // This works for most controls including contenteditable divs.
        match assistant_windows::write_clipboard_text(text) {
            Ok(()) => ToolResult::ok(format!(
                "已将 {} 字符写入剪贴板，请按 Ctrl+V 粘贴",
                text.chars().count()
            )),
            Err(e) => ToolResult::err(format!("写入失败: {e}")),
        }
    }
}
