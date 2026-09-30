//! Tool trait, registry, and cross-platform built-in tools.
//!
//! This crate is platform-agnostic: it defines the `Tool` trait that platform
//! crates (e.g. `lingxi-tools-windows`) implement. The agent engine depends on
//! this crate to get a uniform interface over all available tools.

pub mod builtin;
pub mod context;
pub mod plugin;
pub mod registry;
pub mod schema;

pub use context::{AutoConfirm, ConfirmGate, ConfirmRequest, DenyAll, RiskLevel, ToolContext};
pub use registry::ToolRegistry;
pub use schema::{ToolResult, ToolSchema};

use async_trait::async_trait;

/// A capability that the agent can invoke.
///
/// Each tool is self-describing via [`Tool::schema`] so the LLM can reason about
/// available actions. Tools range from safe read-only operations (reading the
/// clipboard) to dangerous ones (running shell commands); the [`Tool::risk_level`]
/// drives whether the engine asks the user for confirmation before executing.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Metadata exposed to the LLM: name, description, and a JSON Schema for
    /// the parameters object.
    fn schema(&self) -> ToolSchema;

    /// Execute the tool with the given JSON parameters.
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult;

    /// Risk classification. `Safe` tools run without confirmation;
    /// `Moderate` tools warn once per session; `Dangerous` tools always ask.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}
