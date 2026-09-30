//! Action types returned by the model backend during each step of the loop.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One decision from the model: call a tool, reply to the user, or ask
/// for more information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentAction {
    /// The model wants to invoke a tool.
    CallTool {
        /// Provider-assigned call id. It must be echoed on the subsequent
        /// `role=tool` message for OpenAI-compatible APIs.
        id: String,
        name: String,
        arguments: Value,
        /// Optional reasoning text the model produced alongside the call.
        #[serde(skip_serializing_if = "Option::is_none")]
        thought: Option<String>,
    },
    /// The model has a final answer for the user.
    Reply(String),
    /// The model needs more information from the user to proceed.
    AskUser(String),
}

/// A recorded tool call in the session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub result: String,
    pub success: bool,
}
