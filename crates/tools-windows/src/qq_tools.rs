//! QQ chat integration tools.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;

/// Read the user's current text selection inside QQ by scanning the QQ
/// window's UIA tree for a TextPattern element with a non-empty selection.
pub struct QqReadSelectionTool;

#[async_trait]
impl Tool for QqReadSelectionTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "qq_read_selection".into(),
            description:
                "读取QQ中用户当前选中的消息文字。用户需要先在QQ聊天窗口中双击或拖选一条消息。"
                    .into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(&self, _params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        match assistant_windows::capture_qq_selection_text() {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    ToolResult::err("没有选中文字。请在QQ里双击或拖选对方的消息后再试。")
                } else {
                    ToolResult::ok(trimmed.to_string())
                }
            }
            Err(e) => ToolResult::err(format!("读取QQ选区失败: {e}")),
        }
    }
}

/// Write a draft message into the QQ chat composer.
pub struct QqWriteDraftTool;

#[async_trait]
impl Tool for QqWriteDraftTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "qq_write_draft".into(),
            description: "将草稿文本写入QQ聊天输入框（不自动发送）。用户需要检查后手动按发送。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "draft": {"type": "string", "description": "要写入的草稿文本"}
                },
                "required": ["draft"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let draft = match params.get("draft").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => return ToolResult::err("缺少 draft 参数"),
        };

        match assistant_windows::qq_write_draft(draft) {
            Ok(true) => ToolResult::ok("草稿已写入QQ输入框，请检查后按发送。"),
            Ok(false) => ToolResult::ok("草稿已写入（未验证）。"),
            Err(e) => ToolResult::err(format!("写入QQ草稿失败: {e}")),
        }
    }
}
