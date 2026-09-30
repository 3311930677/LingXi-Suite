//! 任务计划（P2-5）：`update_plan` 工具与计划结构（UI 可渲染进度）。
//!
//! 计划存于 `Session.plan`（整表替换语义）；agent 循环在工具执行后对比快照，
//! 变化时发出 [`crate::TurnEvent::PlanUpdate`]，经 SSE `plan_update` 事件透传到前端。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::{Tool, ToolContext, ToolSpec};

/// 工具名。
pub const PLAN_TOOL_NAME: &str = "update_plan";
/// 步骤数量上限。
pub const MAX_PLAN_STEPS: usize = 32;

/// 计划步骤。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub content: String,
    /// `pending` / `in_progress` / `completed`。
    pub status: String,
}

/// 会话级计划（整表替换）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionPlan {
    pub steps: Vec<PlanStep>,
}

impl SessionPlan {
    pub fn pending(&self) -> usize {
        self.count("pending")
    }

    pub fn in_progress(&self) -> usize {
        self.count("in_progress")
    }

    pub fn completed(&self) -> usize {
        self.count("completed")
    }

    fn count(&self, status: &str) -> usize {
        self.steps
            .iter()
            .filter(|step| step.status == status)
            .count()
    }
}

/// `update_plan` 工具：多步任务的结构化计划。
pub struct UpdatePlanTool;

#[async_trait]
impl Tool for UpdatePlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: PLAN_TOOL_NAME.to_string(),
            description: "更新当前任务的步骤计划（整表替换，供界面展示进度）。\
                多步任务应先建立计划；每完成一步调用本工具更新状态，\
                始终提交完整步骤列表而不是增量。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "完整步骤列表（整体替换）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string", "description": "步骤描述" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["steps"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let steps = parse_steps(&args)?;
        let plan = SessionPlan {
            steps: steps.clone(),
        };
        let in_progress = plan.in_progress();
        let pending = plan.pending();
        let completed = plan.completed();
        ctx.session.plan = Some(plan);
        Ok(json!({
            "ok": true,
            "total": steps.len(),
            "pending": pending,
            "in_progress": in_progress,
            "completed": completed,
        }))
    }
}

/// 解析与校验步骤列表（越界/缺字段/非法状态一律结构化报错）。
pub fn parse_steps(args: &Value) -> Result<Vec<PlanStep>, String> {
    let raw = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "缺少 steps 数组".to_string())?;
    if raw.is_empty() {
        return Err("steps 不能为空".to_string());
    }
    if raw.len() > MAX_PLAN_STEPS {
        return Err(format!("steps 最多 {MAX_PLAN_STEPS} 项"));
    }
    let mut steps = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if content.is_empty() {
            return Err(format!("第 {} 步缺少 content", index + 1));
        }
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(status, "pending" | "in_progress" | "completed") {
            return Err(format!("第 {} 步 status 非法：{status}", index + 1));
        }
        steps.push(PlanStep {
            content: content.to_string(),
            status: status.to_string(),
        });
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_invalid_shapes() {
        assert!(parse_steps(&json!({})).is_err());
        assert!(parse_steps(&json!({ "steps": [] })).is_err());
        assert!(parse_steps(&json!({ "steps": [{ "status": "pending" }] })).is_err());
        assert!(parse_steps(&json!({ "steps": [{ "content": "a", "status": "done" }] })).is_err());
    }

    #[test]
    fn parse_accepts_valid_steps() {
        let steps = parse_steps(&json!({
            "steps": [
                { "content": "第一步", "status": "completed" },
                { "content": "第二步", "status": "in_progress" },
                { "content": "第三步", "status": "pending" }
            ]
        }))
        .expect("合法步骤");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].status, "in_progress");
    }

    #[test]
    fn parse_caps_step_count() {
        let many: Vec<Value> = (0..MAX_PLAN_STEPS + 1)
            .map(|index| json!({ "content": format!("step-{index}"), "status": "pending" }))
            .collect();
        assert!(parse_steps(&json!({ "steps": many })).is_err());
    }

    #[test]
    fn plan_counts_by_status() {
        let plan = SessionPlan {
            steps: vec![
                PlanStep {
                    content: "a".to_string(),
                    status: "completed".to_string(),
                },
                PlanStep {
                    content: "b".to_string(),
                    status: "in_progress".to_string(),
                },
                PlanStep {
                    content: "c".to_string(),
                    status: "pending".to_string(),
                },
            ],
        };
        assert_eq!(plan.completed(), 1);
        assert_eq!(plan.in_progress(), 1);
        assert_eq!(plan.pending(), 1);
    }
}
