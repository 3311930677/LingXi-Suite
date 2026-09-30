//! owo-agent 协议镜像（字段与 openapi.json / owo-agent-protocol 对齐）。
//!
//! 全部字段带 `#[serde(default)]`：引擎版本向前演进新增字段时，
//! 旧客户端不会解析失败；未知事件类型落到 [`TurnEvent::Other`]。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `GET /health` 响应。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    #[serde(default)]
    pub healthy: bool,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub auto_approve: bool,
}

/// `POST /session` 请求体。
#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest<'a> {
    pub workspace: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<&'a str>,
}

/// 会话元信息（`SessionInfo`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub fork_point: Option<usize>,
}

/// `POST /session/{id}/turn` 请求体。
#[derive(Debug, Clone, Serialize)]
pub struct TurnRequest {
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<String>,
}

impl TurnRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            attachments: Vec::new(),
        }
    }

    pub fn with_attachments(mut self, attachments: Vec<String>) -> Self {
        self.attachments = attachments;
        self
    }
}

/// `POST /session/{id}/permission/{request_id}` 请求体。
#[derive(Debug, Clone, Serialize)]
pub struct PermissionResponse {
    pub allow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember: Option<bool>,
    /// 记忆作用域（A6-1 三档）：`session` = 本会话内允许；缺省 = 永久规则。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember_scope: Option<String>,
}

/// 文件改动（`GET /session/{id}/diff`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

/// turn SSE 事件（`data` 为带 `v` 版本字段的 tagged JSON）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    Progress {
        #[serde(default)]
        message: String,
    },
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        args: Value,
    },
    ToolResult {
        #[serde(default)]
        id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    PermissionRequest {
        #[serde(default)]
        request_id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        args: Value,
        #[serde(default)]
        reason: String,
    },
    Final {
        #[serde(default)]
        text: String,
    },
    TokenDelta {
        #[serde(default)]
        delta: String,
    },
    Compaction {
        #[serde(default)]
        summary: String,
    },
    /// ask_user 提问：引擎挂起等待用户回答（UI 应展示提问卡并切换到等待态）。
    UserQuestion {
        #[serde(default)]
        question_id: String,
        #[serde(default)]
        question: String,
        #[serde(default)]
        options: Vec<String>,
    },
    /// 提问结束（用户已回答 / 超时 / 中止）。
    UserAnswered {
        #[serde(default)]
        question_id: String,
        #[serde(default)]
        answer: String,
        #[serde(default)]
        source: String,
    },
    /// 未知事件类型（服务端新增事件时的向前兼容；UI 可忽略）。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn turn_event_parses_tagged_payload_with_version_field() {
        let raw = r#"{"type":"token_delta","delta":"你好","v":1}"#;
        let event: TurnEvent = serde_json::from_str(raw).unwrap();
        assert_eq!(
            event,
            TurnEvent::TokenDelta {
                delta: "你好".to_string()
            }
        );
    }

    #[test]
    fn turn_event_tolerates_missing_fields_and_unknown_kinds() {
        let event: TurnEvent = serde_json::from_str(r#"{"type":"tool_result","v":1}"#).unwrap();
        assert_eq!(
            event,
            TurnEvent::ToolResult {
                id: String::new(),
                tool: String::new(),
                ok: false,
                error: None,
            }
        );
        let event: TurnEvent =
            serde_json::from_str(r#"{"type":"brand_new_event","v":1,"x":1}"#).unwrap();
        assert_eq!(event, TurnEvent::Other);
    }

    #[test]
    fn permission_request_round_trips_engine_fields() {
        let raw = json!({
            "type": "permission_request",
            "request_id": "req-1",
            "tool": "edit_file",
            "args": {"path": "a.txt"},
            "reason": "write 文件操作（工作区内）",
            "v": 1
        })
        .to_string();
        let event: TurnEvent = serde_json::from_str(&raw).unwrap();
        match event {
            TurnEvent::PermissionRequest {
                request_id,
                tool,
                args,
                reason,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(tool, "edit_file");
                assert_eq!(args["path"], "a.txt");
                assert!(reason.contains("工作区内"));
            }
            other => panic!("解析成了 {other:?}"),
        }
    }

    #[test]
    fn request_bodies_match_engine_contract() {
        let create = serde_json::to_value(CreateSessionRequest {
            workspace: "D:/work",
            model: None,
            system_prompt: None,
        })
        .unwrap();
        assert_eq!(create, json!({"workspace": "D:/work"}));

        let turn = serde_json::to_value(TurnRequest::new("改个 bug")).unwrap();
        assert_eq!(turn, json!({"prompt": "改个 bug", "attachments": []}));

        let permission = serde_json::to_value(PermissionResponse {
            allow: true,
            remember: None,
            remember_scope: None,
        })
        .unwrap();
        assert_eq!(permission, json!({"allow": true}));
    }
}
