//! Agent backend trait: the interface between the engine and the model.

use crate::action::AgentAction;
use crate::error::AgentError;
use crate::session::Message;
use async_trait::async_trait;
use lingxi_tools::ToolSchema;

/// A model backend that can decide which tool to call next.
///
/// Implementations include:
/// - `CloudAgentBackend` (in `assistant-inference`): uses OpenAI function calling.
/// - `ReActBackend` (future): parses `Action: ...` text from local models.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Given the conversation history and the list of available tool schemas,
    /// return the model's next action (call a tool, reply, or ask the user).
    async fn step(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<AgentAction, AgentError>;
}
