//! Persistent reminder queue. Notification delivery is owned by the host UI.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_MESSAGE_CHARS: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Reminder {
    id: String,
    due_at: i64,
    message: String,
    cancelled: bool,
}

pub struct SetReminderTool;
pub struct ListRemindersTool;
pub struct CancelReminderTool;

#[async_trait]
impl Tool for SetReminderTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "set_reminder".into(),
            description:
                "设置一个持久化提醒。使用 Unix 秒时间戳 due_at，或使用 delay_seconds 设置相对时间。"
                    .into(),
            parameters: json!({"type":"object","properties":{"due_at":{"type":"integer","description":"到期时间的 Unix 秒时间戳"},"delay_seconds":{"type":"integer","minimum":1,"description":"从现在起延迟的秒数"},"message":{"type":"string","description":"提醒内容"}},"required":["message"]}),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let message = match params.get("message").and_then(|v| v.as_str()) {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => return ToolResult::err("message 不能为空"),
        };
        if message.chars().count() > MAX_MESSAGE_CHARS {
            return ToolResult::err(format!("message 最多支持 {MAX_MESSAGE_CHARS} 个字符"));
        }
        let now = unix_now();
        let due_at = match (
            params.get("due_at").and_then(|v| v.as_i64()),
            params.get("delay_seconds").and_then(|v| v.as_i64()),
        ) {
            (Some(value), _) if value > now => value,
            (_, Some(value)) if value > 0 => now.saturating_add(value),
            _ => return ToolResult::err("必须提供未来的 due_at，或正数 delay_seconds"),
        };
        let reminder = Reminder {
            id: format!("r-{}", now_nanos()),
            due_at,
            message,
            cancelled: false,
        };
        if let Err(error) = append(&path(&ctx.working_dir), &reminder) {
            return ToolResult::err(format!("保存提醒失败: {error}"));
        }
        ToolResult::ok_with_data(
            format!(
                "提醒已设置，ID: {}，到期时间 Unix: {}",
                reminder.id, reminder.due_at
            ),
            json!(reminder),
        )
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }
}

#[async_trait]
impl Tool for ListRemindersTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_reminders".into(),
            description: "列出尚未取消的提醒，可筛选已到期提醒。".into(),
            parameters: json!({"type":"object","properties":{"include_expired":{"type":"boolean"}}}),
        }
    }
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let include_expired = params
            .get("include_expired")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let now = unix_now();
        match read_all(&path(&ctx.working_dir)) {
            Ok(items) => {
                let items: Vec<_> = items
                    .into_iter()
                    .filter(|r| !r.cancelled && (include_expired || r.due_at > now))
                    .collect();
                ToolResult::ok_with_data(format!("当前有 {} 条提醒", items.len()), json!(items))
            }
            Err(error) => ToolResult::err(format!("读取提醒失败: {error}")),
        }
    }
}

#[async_trait]
impl Tool for CancelReminderTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "cancel_reminder".into(),
            description: "按 ID 取消一个提醒。".into(),
            parameters: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
        }
    }
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(value) if !value.is_empty() => value,
            _ => return ToolResult::err("id 不能为空"),
        };
        let file = path(&ctx.working_dir);
        let mut items = match read_all(&file) {
            Ok(items) => items,
            Err(error) => return ToolResult::err(format!("读取提醒失败: {error}")),
        };
        let Some(item) = items
            .iter_mut()
            .find(|item| item.id == id && !item.cancelled)
        else {
            return ToolResult::err(format!("未找到提醒: {id}"));
        };
        item.cancelled = true;
        if let Err(error) = rewrite(&file, &items) {
            return ToolResult::err(format!("更新提醒失败: {error}"));
        }
        ToolResult::ok(format!("已取消提醒 {id}"))
    }
}

fn path(root: &Path) -> PathBuf {
    root.join(".lingxi").join("reminders.jsonl")
}
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
fn append(file: &Path, reminder: &Reminder) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut out, reminder).map_err(|e| e.to_string())?;
    out.write_all(b"\n").map_err(|e| e.to_string())
}
fn read_all(file: &Path) -> Result<Vec<Reminder>, String> {
    if !file.exists() {
        return Ok(Vec::new());
    }
    BufReader::new(fs::File::open(file).map_err(|e| e.to_string())?)
        .lines()
        .map(|line| {
            let line = line.map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())
        })
        .collect()
}
fn rewrite(file: &Path, items: &[Reminder]) -> Result<(), String> {
    let temp = file.with_extension("tmp");
    let mut out = fs::File::create(&temp).map_err(|e| e.to_string())?;
    for item in items {
        serde_json::to_writer(&mut out, item).map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    fs::rename(temp, file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_root() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lingxi-reminder-test-{}-{}-{}",
            std::process::id(),
            now_nanos(),
            n
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    async fn set_ok(root: &Path, params: serde_json::Value) -> Reminder {
        let ctx = ToolContext::auto_confirm(root);
        let result = SetReminderTool.execute(params, &ctx).await;
        assert!(result.success, "set 应成功: {}", result.output);
        serde_json::from_value(result.data.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn set_reminder_persists_with_delay() {
        let root = temp_root();
        let reminder = set_ok(&root, json!({"message": "喝水", "delay_seconds": 3600})).await;
        assert!(reminder.id.starts_with("r-"));
        assert_eq!(reminder.message, "喝水");
        assert!(!reminder.cancelled);
        assert!(reminder.due_at > unix_now());
        assert!(reminder.due_at <= unix_now() + 3600);
        let stored = read_all(&path(&root)).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, reminder.id);
        cleanup(&root);
    }

    #[tokio::test]
    async fn set_reminder_validates_message() {
        let root = temp_root();
        let ctx = ToolContext::auto_confirm(&root);
        let tool = SetReminderTool;
        for bad in [json!({}), json!({"message": ""}), json!({"message": "   "})] {
            let result = tool.execute(bad, &ctx).await;
            assert!(!result.success);
            assert!(result.output.contains("message 不能为空"));
        }
        let overlong: String = "字".repeat(1001);
        let result = tool
            .execute(json!({"message": overlong, "delay_seconds": 60}), &ctx)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("最多支持 1000 个字符"));
        let boundary: String = "字".repeat(1000);
        let result = tool
            .execute(json!({"message": boundary, "delay_seconds": 60}), &ctx)
            .await;
        assert!(result.success, "恰好 1000 字应成功: {}", result.output);
        assert!(!result.output.contains("最多支持"));
        cleanup(&root);
    }

    #[tokio::test]
    async fn set_reminder_validates_due_time() {
        let root = temp_root();
        let ctx = ToolContext::auto_confirm(&root);
        let tool = SetReminderTool;
        let past = unix_now() - 10;
        for bad in [
            json!({"message": "x", "due_at": past}),
            json!({"message": "x"}),
            json!({"message": "x", "delay_seconds": 0}),
            json!({"message": "x", "delay_seconds": -5}),
        ] {
            let result = tool.execute(bad, &ctx).await;
            assert!(!result.success);
            assert!(result.output.contains("必须提供未来的 due_at"));
        }
        let due = unix_now() + 120;
        let result = tool
            .execute(json!({"message": "x", "due_at": due}), &ctx)
            .await;
        assert!(result.success);
        let reminder: Reminder = serde_json::from_value(result.data.unwrap()).unwrap();
        assert_eq!(reminder.due_at, due);
        cleanup(&root);
    }

    #[tokio::test]
    async fn set_reminder_prefers_future_due_at_over_delay() {
        let root = temp_root();
        let due = unix_now() + 300;
        let reminder = set_ok(
            &root,
            json!({"message": "双参", "due_at": due, "delay_seconds": 999_999}),
        )
        .await;
        assert_eq!(reminder.due_at, due);
        cleanup(&root);
    }

    #[tokio::test]
    async fn list_reminders_empty_store() {
        let root = temp_root();
        let ctx = ToolContext::auto_confirm(&root);
        let result = ListRemindersTool.execute(json!({}), &ctx).await;
        assert!(result.success);
        assert!(result.output.contains("0 条提醒"));
        assert_eq!(result.data.unwrap().as_array().unwrap().len(), 0);
        cleanup(&root);
    }

    #[tokio::test]
    async fn list_reminders_filters_cancelled_and_expired() {
        let root = temp_root();
        let past = json!({"id": "r-past", "due_at": unix_now() - 100, "message": "已过期", "cancelled": false});
        let cancelled = json!({"id": "r-cancelled", "due_at": unix_now() + 999, "message": "已取消", "cancelled": true});
        fs::create_dir_all(path(&root).parent().unwrap()).unwrap();
        {
            let mut out = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path(&root))
                .unwrap();
            serde_json::to_writer(&mut out, &past).unwrap();
            out.write_all(b"\n").unwrap();
            serde_json::to_writer(&mut out, &cancelled).unwrap();
            out.write_all(b"\n").unwrap();
        }
        set_ok(&root, json!({"message": "未来", "delay_seconds": 3600})).await;

        let ctx = ToolContext::auto_confirm(&root);
        let tool = ListRemindersTool;
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.success);
        assert_eq!(result.data.unwrap().as_array().unwrap().len(), 2);

        let result = tool.execute(json!({"include_expired": true}), &ctx).await;
        assert_eq!(result.data.unwrap().as_array().unwrap().len(), 2);

        let result = tool.execute(json!({"include_expired": false}), &ctx).await;
        let active = result.data.unwrap().as_array().unwrap().clone();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0]["message"], json!("未来"));
        assert!(active[0]["due_at"].as_i64().unwrap() > unix_now());
        cleanup(&root);
    }

    #[tokio::test]
    async fn cancel_reminder_flow() {
        let root = temp_root();
        let first = set_ok(&root, json!({"message": "一", "delay_seconds": 600})).await;
        let second = set_ok(&root, json!({"message": "二", "delay_seconds": 600})).await;

        let ctx = ToolContext::auto_confirm(&root);
        let tool = CancelReminderTool;
        let result = tool.execute(json!({"id": first.id}), &ctx).await;
        assert!(result.success, "{}", result.output);
        assert!(result.output.contains(&first.id));

        let stored = read_all(&path(&root)).unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored.iter().find(|r| r.id == first.id).unwrap().cancelled);
        assert!(!stored.iter().find(|r| r.id == second.id).unwrap().cancelled);
        assert!(!path(&root).with_extension("tmp").exists());

        for missing in ["r-nope", first.id.as_str()] {
            let result = tool.execute(json!({"id": missing}), &ctx).await;
            assert!(!result.success);
            assert!(result.output.contains("未找到提醒"));
        }
        let result = tool.execute(json!({"id": ""}), &ctx).await;
        assert!(!result.success);
        assert!(result.output.contains("id 不能为空"));

        let result = ListRemindersTool.execute(json!({}), &ctx).await;
        let items = result.data.unwrap().as_array().unwrap().clone();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], json!(second.id));
        cleanup(&root);
    }
}
