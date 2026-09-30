//! Execution context passed to every tool invocation, plus the confirmation
//! gate that controls dangerous operations.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Risk classification for tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Read-only operations: reading files, listing windows, searching.
    Safe,
    /// Write operations that modify the user's environment but are reversible:
    /// writing text to a control, clipboard, etc.
    Moderate,
    /// Irreversible or high-impact operations: running shell commands,
    /// deleting files, sending messages. Always requires explicit confirmation.
    Dangerous,
}

/// A request for user confirmation before a dangerous operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmRequest {
    /// Name of the tool requesting confirmation.
    pub tool_name: String,
    /// Short human-readable summary of what will happen.
    pub action_summary: String,
    /// Risk level of the operation.
    pub risk_level: RiskLevel,
    /// The parameters that will be passed to the tool.
    pub params: serde_json::Value,
}

/// Trait for confirmation gates. The agent engine calls `confirm` before
/// executing any tool whose risk level warrants it.
pub trait ConfirmGate: Send + Sync {
    fn confirm(&self, request: &ConfirmRequest) -> bool;
}

/// A confirmation gate that allows everything. Used in tests and headless mode.
pub struct AutoConfirm;

impl ConfirmGate for AutoConfirm {
    fn confirm(&self, _request: &ConfirmRequest) -> bool {
        true
    }
}

/// A confirmation gate that denies everything. Used when running in a
/// restricted mode where no dangerous operations are allowed.
pub struct DenyAll;

impl ConfirmGate for DenyAll {
    fn confirm(&self, _request: &ConfirmRequest) -> bool {
        false
    }
}

/// Context passed to `Tool::execute`. It carries the session ID, a working
/// directory for file operations, and a reference to the confirmation gate.
///
/// Owns an `Arc<dyn ConfirmGate>` to avoid lifetime parameters that would
/// complicate the `async_trait` desugaring of the `Tool` trait.
pub struct ToolContext {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub confirm: Arc<dyn ConfirmGate>,
}

impl ToolContext {
    /// Create a context with the given working directory and auto-confirm.
    /// Useful for tests and headless tool invocations.
    pub fn auto_confirm(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            session_id: "test".to_string(),
            working_dir: working_dir.into(),
            confirm: Arc::new(AutoConfirm),
        }
    }

    /// Create a context with a deny-all gate (no dangerous ops allowed).
    pub fn deny_all(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            session_id: "test".to_string(),
            working_dir: working_dir.into(),
            confirm: Arc::new(DenyAll),
        }
    }
}
