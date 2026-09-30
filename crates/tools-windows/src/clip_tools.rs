//! Clipboard read/write tools.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;

/// Read the current clipboard text.
pub struct ReadClipboardTool;

#[async_trait]
impl Tool for ReadClipboardTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_clipboard".into(),
            description: "读取当前剪贴板中的文本内容。".into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(&self, _params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        match assistant_windows::read_clipboard_text() {
            Ok(text) => ToolResult::ok(text),
            Err(e) => ToolResult::err(format!("读取剪贴板失败: {e}")),
        }
    }
}

/// Write text to the clipboard.
pub struct WriteClipboardTool;

#[async_trait]
impl Tool for WriteClipboardTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_clipboard".into(),
            description: "设置剪贴板内容为指定文本。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "要写入剪贴板的文本"}
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

        match assistant_windows::write_clipboard_text(text) {
            Ok(()) => ToolResult::ok("剪贴板已更新"),
            Err(e) => ToolResult::err(format!("写入剪贴板失败: {e}")),
        }
    }
}
