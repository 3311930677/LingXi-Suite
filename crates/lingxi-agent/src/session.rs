//! Session: conversation history and tool-call log.

use crate::action::ToolCall;
use serde::{Deserialize, Serialize};

/// Message role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// A tool result message (OpenAI format: role = "tool").
    Tool,
}

/// A single message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls attached to an assistant message (for function calling).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    /// Name of the tool that produced this message (for role = Tool).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_name: Option<String>,
    /// Provider call id referenced by a role=tool result.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: result.into(),
            tool_calls: vec![],
            tool_name: Some(name.into()),
            tool_call_id: Some(call_id.into()),
        }
    }

    /// Serialize this message in OpenAI-compatible chat-completions format.
    pub fn to_openai_json(&self) -> serde_json::Value {
        let role = match self.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let mut obj = serde_json::json!({
            "role": role,
            "content": self.content,
        });
        if let Some(ref name) = self.tool_name {
            obj["name"] = serde_json::Value::String(name.clone());
        }
        if let Some(ref call_id) = self.tool_call_id {
            obj["tool_call_id"] = serde_json::Value::String(call_id.clone());
        }
        if !self.tool_calls.is_empty() {
            obj["tool_calls"] = serde_json::Value::Array(
                self.tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            }
                        })
                    })
                    .collect(),
            );
        }
        obj
    }
}

/// A conversation session with history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    /// Working directory for file-based tools.
    pub working_dir: std::path::PathBuf,
}

impl Default for Session {
    fn default() -> Self {
        Self::new("default", ".")
    }
}

impl Session {
    pub fn new(id: impl Into<String>, working_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            id: id.into(),
            messages: vec![],
            working_dir: working_dir.into(),
        }
    }

    /// Add the system prompt once, before the first user turn.
    pub fn ensure_system(&mut self, content: impl Into<String>) {
        if !self
            .messages
            .iter()
            .any(|message| message.role == Role::System)
        {
            self.messages.insert(0, Message::system(content));
        }
    }

    pub fn push_user(&mut self, content: impl Into<String>) {
        self.messages.push(Message::user(content));
    }

    pub fn push_assistant(&mut self, content: impl Into<String>) {
        self.messages.push(Message::assistant(content));
    }

    pub fn push_assistant_with_tools(&mut self, content: impl Into<String>, calls: Vec<ToolCall>) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: calls,
            tool_name: None,
            tool_call_id: None,
        });
    }

    pub fn push_tool_result(&mut self, call_id: &str, tool_name: &str, result: &str) {
        self.messages
            .push(Message::tool_result(call_id, tool_name, result));
    }

    /// Bound the conversation history while preserving the system prompt and
    /// complete assistant-tool message pairs. This prevents an unattended
    /// desktop session from growing the request payload without limit.
    pub fn trim_history(&mut self, max_non_system_messages: usize) {
        let system = self
            .messages
            .iter()
            .find(|message| message.role == Role::System)
            .cloned();
        let non_system: Vec<_> = self
            .messages
            .iter()
            .filter(|message| message.role != Role::System)
            .cloned()
            .collect();
        if non_system.len() <= max_non_system_messages {
            return;
        }

        let mut start = non_system.len() - max_non_system_messages;
        // A tool result must retain the immediately preceding assistant call.
        if non_system
            .get(start)
            .is_some_and(|message| message.role == Role::Tool)
        {
            start = start.saturating_sub(1);
        }
        self.messages.clear();
        if let Some(system) = system {
            self.messages.push(system);
        }
        self.messages.extend(non_system.into_iter().skip(start));
    }

    /// All messages as serde_json values, suitable for the OpenAI API.
    pub fn messages_json(&self) -> Vec<serde_json::Value> {
        self.messages.iter().map(Message::to_openai_json).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_tool_messages_preserve_call_id() {
        let mut session = Session::new("test", ".");
        session.push_assistant_with_tools(
            "读取文件",
            vec![ToolCall {
                id: "call_42".into(),
                name: "read_file".into(),
                arguments: json!({"path": "README.md"}),
                result: String::new(),
                success: false,
            }],
        );
        session.push_tool_result("call_42", "read_file", "content");
        let json = session.messages_json();
        assert_eq!(json[0]["tool_calls"][0]["id"], "call_42");
        assert_eq!(json[1]["tool_call_id"], "call_42");
    }

    #[test]
    fn trim_history_keeps_system_and_complete_tool_pair() {
        let mut session = Session::new("test", ".");
        session.ensure_system("system");
        for index in 0..5 {
            session.push_user(format!("message-{index}"));
        }
        session.push_assistant_with_tools(
            "call",
            vec![ToolCall {
                id: "call_last".into(),
                name: "echo".into(),
                arguments: json!({}),
                result: String::new(),
                success: false,
            }],
        );
        session.push_tool_result("call_last", "echo", "ok");
        session.trim_history(1);
        assert_eq!(session.messages[0].role, Role::System);
        assert_eq!(session.messages[1].role, Role::Assistant);
        assert_eq!(session.messages[2].role, Role::Tool);
    }
}
