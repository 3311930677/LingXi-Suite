//! Agent SDK 公开线协议（v1 契约，HTTP JSON + SSE 事件）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: String,
    pub model: String,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub fork_point: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRequest {
    pub prompt: String,
    /// 附件 ID（由 `POST /session/{id}/attachments` 返回；发送时注入路径上下文）。
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkRequest {
    pub message_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindRequest {
    pub keep: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRunRequest {
    pub suite_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub allow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember: Option<bool>,
    /// 记忆作用域（A6-1 三档授权）：`session` = 本会话内允许（内存临时规则，
    /// 不落盘、重启失效）；缺省或 `forever` = 永久规则（写入 settings.json）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub healthy: bool,
    pub version: String,
    pub auto_approve: bool,
}

/// SSE 事件协议版本（R10：所有 SSE 事件帧 data 统一携带 `v` 字段）。
/// 变更策略：破坏性事件结构变更 → 递增版本并登记 RFC 注释（弃用期 ≥2 个 minor）。
pub const SSE_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Progress {
        message: String,
    },
    ToolUse {
        id: String,
        tool: String,
        args: Value,
    },
    ToolResult {
        id: String,
        tool: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// 结果预览（截断）：前端步骤 chip 展开时展示「这一步到底产出了什么」。
        /// 附加可选字段（非破坏性变更，SSE_PROTOCOL_VERSION 不变）；
        /// 老客户端忽略该字段即可，完整结果仍以会话记录为准。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    PermissionRequest {
        request_id: String,
        tool: String,
        args: Value,
        reason: String,
    },
    /// 审批已解决（B1-5 收尾，与提问的 `UserAnswered` 对称）：前端关闭审批卡
    /// 行动区并显示结果。此前只有 `PermissionRequest`，300s 超时/回合中止后
    /// 卡片仍留在界面上可点，用户点了只会得到无意义的回传错误。
    PermissionResolved {
        request_id: String,
        /// 来源：`user`=用户点了允许/拒绝；`timeout`=300s 未响应；`aborted`=回合中止。
        #[serde(default)]
        source: String,
        /// 最终是否放行（超时/中止一律按拒绝）。
        #[serde(default)]
        allowed: bool,
    },
    Final {
        text: String,
    },
    /// 回合失败终态（异常/中断）：必须显式下发，避免前端停留在「执行中」。
    TurnFailed {
        message: String,
    },
    /// 回合统计（`run_turn` 结束后补发）：供前端回合汇报卡展示耗时/步数/消耗。
    TurnStats {
        steps: usize,
        duration_ms: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        cost_usd: f64,
    },
    TokenDelta {
        delta: String,
    },
    /// 深度思考增量（模型 reasoning；不写入对话历史）。
    ReasoningDelta {
        delta: String,
    },
    Compaction {
        summary: String,
    },
    /// 模型请求向用户提问（ask_user 工具）：前端展示提问卡，回合挂起等待回答。
    /// 用户通过 `POST /session/{id}/answer/{question_id}` 提交答案。
    UserQuestion {
        question_id: String,
        question: String,
        #[serde(default)]
        options: Vec<String>,
    },
    /// 用户已回答（或提问超时/中止）：前端关闭提问卡并回显答案。
    UserAnswered {
        question_id: String,
        answer: String,
        /// 回答来源：user=用户提交；timeout=超时未答；aborted=回合中止。
        #[serde(default)]
        source: String,
    },
    /// 任务计划更新（P2-5，`update_plan` 工具）：前端渲染步骤进度。
    PlanUpdate {
        /// `[{ "content": "...", "status": "pending|in_progress|completed" }]`。
        steps: Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B1-5：审批终态事件的线上表示（tag + 可选字段缺省），前端按 type 分派。
    #[test]
    fn permission_resolved_serializes_with_tag_and_defaults() {
        let event = SseEvent::PermissionResolved {
            request_id: "req-1".to_string(),
            source: "timeout".to_string(),
            allowed: false,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "permission_resolved");
        assert_eq!(value["request_id"], "req-1");
        assert_eq!(value["source"], "timeout");
        assert_eq!(value["allowed"], false);

        // 反序列化：source / allowed 缺省（旧发送端或最小帧）。
        let parsed: SseEvent = serde_json::from_value(serde_json::json!({
            "type": "permission_resolved",
            "request_id": "req-2"
        }))
        .unwrap();
        match parsed {
            SseEvent::PermissionResolved {
                request_id,
                source,
                allowed,
            } => {
                assert_eq!(request_id, "req-2");
                assert!(source.is_empty());
                assert!(!allowed);
            }
            other => panic!("应为 PermissionResolved：{other:?}"),
        }
    }
}