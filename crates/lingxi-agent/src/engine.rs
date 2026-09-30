//! The agent engine: orchestrates the think → act → observe loop.

use crate::action::{AgentAction, ToolCall};
use crate::backend::AgentBackend;
use crate::error::AgentError;
use crate::session::Session;
use lingxi_tools::{ConfirmGate, ToolContext, ToolRegistry};
use std::sync::Arc;

const DEFAULT_SYSTEM_PROMPT: &str = r#"你是灵犀，一个运行在用户 Windows 桌面上的 AI 协作助手。

工作原则：
1. 优先理解用户真实目标，必要时组合多个工具完成任务。
2. 只调用完成当前任务所必需的工具；工具失败时根据返回结果调整，不要假装成功。
3. 不要自动发送聊天消息；QQ 工具只写草稿，由用户最终确认发送。
4. 文件写入、命令执行等危险能力可能被安全策略禁用，遇到拒绝时明确说明并给出安全替代方案。
5. 最终回复简洁说明结果、改动位置和用户需要继续做的事。"#;

/// Detailed result of one agent turn, including every tool invocation for UI
/// rendering and auditability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRunReport {
    pub reply: String,
    pub tool_calls: Vec<ToolCall>,
}

/// The agent engine. Holds a backend (model) and a tool registry.
///
/// The main method is [`AgentEngine::run`] which executes the ReAct loop:
/// ask the model → if it wants a tool, execute it → feed the result back →
/// repeat until the model replies or the step limit is reached.
pub struct AgentEngine {
    backend: Box<dyn AgentBackend>,
    registry: std::sync::Arc<ToolRegistry>,
    max_steps: usize,
}

impl AgentEngine {
    pub fn new(backend: Box<dyn AgentBackend>, registry: std::sync::Arc<ToolRegistry>) -> Self {
        Self {
            backend,
            registry,
            max_steps: 10,
        }
    }

    pub fn with_max_steps(mut self, max: usize) -> Self {
        self.max_steps = max;
        self
    }

    /// Run one round of conversation and return only the final reply.
    pub async fn run(
        &self,
        user_input: &str,
        session: &mut Session,
        confirm: Arc<dyn ConfirmGate>,
    ) -> Result<String, AgentError> {
        self.run_with_trace(user_input, session, confirm)
            .await
            .map(|report| report.reply)
    }

    /// Run one round and retain a structured audit trail of tool calls.
    pub async fn run_with_trace(
        &self,
        user_input: &str,
        session: &mut Session,
        confirm: Arc<dyn ConfirmGate>,
    ) -> Result<AgentRunReport, AgentError> {
        session.ensure_system(DEFAULT_SYSTEM_PROMPT);
        session.push_user(user_input);
        session.trim_history(40);
        let mut trace = Vec::new();

        for _step in 0..self.max_steps {
            let action = self
                .backend
                .step(&session.messages, &self.registry.enabled_schemas())
                .await?;

            match action {
                AgentAction::Reply(text) | AgentAction::AskUser(text) => {
                    session.push_assistant(&text);
                    return Ok(AgentRunReport {
                        reply: text,
                        tool_calls: trace,
                    });
                }
                AgentAction::CallTool {
                    id,
                    name,
                    arguments,
                    thought,
                } => {
                    // Persist the exact assistant tool call before executing it.
                    // OpenAI-compatible APIs require this message immediately
                    // before the role=tool result carrying the same call id.
                    let protocol_call = ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                        result: String::new(),
                        success: false,
                    };
                    session.push_assistant_with_tools(
                        thought.unwrap_or_default(),
                        vec![protocol_call],
                    );

                    let ctx = ToolContext {
                        session_id: session.id.clone(),
                        working_dir: session.working_dir.clone(),
                        confirm: confirm.clone(),
                    };

                    let result = match self.registry.execute(&name, arguments.clone(), &ctx).await {
                        Some(result) => result,
                        None => lingxi_tools::ToolResult::err(format!("未知工具: {name}")),
                    };
                    session.push_tool_result(&id, &name, &result.output);
                    trace.push(ToolCall {
                        id,
                        name,
                        arguments,
                        result: result.output,
                        success: result.success,
                    });
                }
            }
        }

        Err(AgentError::MaxStepsExceeded(self.max_steps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::AgentAction;
    use crate::backend::AgentBackend;
    use crate::session::{Message, Session};
    use async_trait::async_trait;
    use lingxi_tools::context::AutoConfirm;
    use lingxi_tools::{Tool, ToolResult, ToolSchema};
    use serde_json::json;
    use std::sync::Arc;

    /// A mock backend that replays a scripted sequence of actions.
    struct MockBackend {
        actions: std::sync::Mutex<Vec<AgentAction>>,
    }

    impl MockBackend {
        fn new(actions: Vec<AgentAction>) -> Self {
            Self {
                actions: std::sync::Mutex::new(actions),
            }
        }
    }

    #[async_trait]
    impl AgentBackend for MockBackend {
        async fn step(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> Result<AgentAction, AgentError> {
            let mut actions = self.actions.lock().unwrap();
            if actions.is_empty() {
                Ok(AgentAction::Reply("（无更多动作）".into()))
            } else {
                Ok(actions.remove(0))
            }
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "echo".into(),
                description: "Echoes text back.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
            }
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &lingxi_tools::ToolContext,
        ) -> ToolResult {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            ToolResult::ok(text.to_string())
        }
    }

    #[tokio::test]
    async fn agent_calls_tool_then_replies() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let backend = MockBackend::new(vec![
            AgentAction::CallTool {
                id: "call_echo_1".into(),
                name: "echo".into(),
                arguments: json!({"text": "hello world"}),
                thought: Some("我需要回显这段文字".into()),
            },
            AgentAction::Reply("工具返回了: hello world".into()),
        ]);

        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry));
        let mut session = Session::new("test", ".");

        let report = engine
            .run_with_trace(
                "echo hello world",
                &mut session,
                Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>,
            )
            .await
            .unwrap();

        assert_eq!(report.reply, "工具返回了: hello world");
        assert_eq!(report.tool_calls.len(), 1);
        assert_eq!(report.tool_calls[0].id, "call_echo_1");
        assert_eq!(report.tool_calls[0].result, "hello world");
        assert!(report.tool_calls[0].success);
        // system + user + assistant_tool_call + tool_result + reply = 5 messages
        assert_eq!(session.messages.len(), 5);
        let json = session.messages_json();
        assert_eq!(json[2]["tool_calls"][0]["id"], "call_echo_1");
        assert_eq!(json[3]["tool_call_id"], "call_echo_1");
    }

    #[tokio::test]
    async fn agent_replies_directly() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let backend = MockBackend::new(vec![AgentAction::Reply("直接回答".into())]);
        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry));
        let mut session = Session::new("test", ".");

        let reply = engine
            .run(
                "你好",
                &mut session,
                Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>,
            )
            .await
            .unwrap();

        assert_eq!(reply, "直接回答");
        // system + user + assistant_reply = 3 messages
        assert_eq!(session.messages.len(), 3);
    }

    #[tokio::test]
    async fn agent_exceeds_max_steps() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        // Every step calls the tool, never replies.
        let backend = MockBackend::new(
            (0..20)
                .enumerate()
                .map(|(index, _)| AgentAction::CallTool {
                    id: format!("call_loop_{index}"),
                    name: "echo".into(),
                    arguments: json!({"text": "loop"}),
                    thought: None,
                })
                .collect(),
        );

        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry)).with_max_steps(3);
        let mut session = Session::new("test", ".");

        let result = engine
            .run(
                "loop",
                &mut session,
                Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>,
            )
            .await;
        assert!(matches!(result, Err(AgentError::MaxStepsExceeded(3))));
    }
}
