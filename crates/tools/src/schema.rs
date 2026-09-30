//! Tool metadata and result types shared across all tools.

use serde::{Deserialize, Serialize};

/// JSON-Schema-based metadata describing a tool to the LLM.
///
/// The `parameters` field is a JSON Schema object compatible with OpenAI's
/// function-calling format. The registry collects all schemas and passes them
/// to the backend so the model knows which tools are available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Machine-readable name, e.g. `"read_file"`.
    pub name: String,
    /// Human/LLM-readable description of what the tool does.
    pub description: String,
    /// JSON Schema for the `arguments` object. Use `"type": "object"` with
    /// `properties` / `required` as needed.
    pub parameters: serde_json::Value,
}

/// The outcome of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool completed successfully.
    pub success: bool,
    /// Text returned to the LLM. On failure this is an error message.
    pub output: String,
    /// Optional structured data for the UI to render (e.g. a file listing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ToolResult {
    /// Shorthand for a successful text result.
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            success: true,
            output: text.into(),
            data: None,
        }
    }

    /// Shorthand for a failure with an error message.
    pub fn err(text: impl Into<String>) -> Self {
        Self {
            success: false,
            output: text.into(),
            data: None,
        }
    }

    /// Shorthand for success with structured data.
    pub fn ok_with_data(text: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: true,
            output: text.into(),
            data: Some(data),
        }
    }
}
