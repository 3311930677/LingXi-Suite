//! Tool registry: register tools by name, list schemas, and route execution.

use crate::context::{ConfirmRequest, RiskLevel, ToolContext};
use crate::schema::{ToolResult, ToolSchema};
use crate::Tool;
use std::collections::HashMap;
use std::sync::Arc;

/// Central registry of all available tools. The agent engine queries it for
/// schemas (to send to the LLM) and routes tool calls through `execute`.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Per-tool enable flag. Disabled tools are hidden from the LLM and
    /// cannot be called. This lets the user turn off specific capabilities.
    enabled: HashMap<String, bool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            enabled: HashMap::new(),
        }
    }

    /// Register a tool. It is enabled by default.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.schema().name;
        self.enabled.insert(name.clone(), true);
        self.tools.insert(name, tool);
    }

    /// Enable or disable a tool by name.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        self.enabled.insert(name.to_string(), enabled);
    }

    /// Unregister a tool by name (used when a plugin is uninstalled).
    /// Returns true if the tool was present.
    pub fn remove(&mut self, name: &str) -> bool {
        self.enabled.remove(name);
        self.tools.remove(name).is_some()
    }

    /// Check whether a tool is currently enabled.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.get(name).copied().unwrap_or(false)
    }

    /// All registered tool schemas (including disabled ones, for UI display).
    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        let mut schemas: Vec<_> = self.tools.values().map(|tool| tool.schema()).collect();
        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        schemas
    }

    /// Only enabled tool schemas, for sending to the LLM. The stable name
    /// ordering keeps prompts deterministic across runs despite HashMap's
    /// randomized iteration order.
    pub fn enabled_schemas(&self) -> Vec<ToolSchema> {
        let mut schemas: Vec<_> = self
            .tools
            .iter()
            .filter(|(name, _)| self.is_enabled(name))
            .map(|(_, tool)| tool.schema())
            .collect();
        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        schemas
    }

    /// Get the risk level of a tool, if it exists.
    pub fn risk_of(&self, name: &str) -> Option<RiskLevel> {
        self.tools.get(name).map(|t| t.risk_level())
    }

    /// Execute a tool by name, after checking the confirmation gate.
    ///
    /// Returns `None` if the tool is not registered or is disabled.
    /// Returns a `ToolResult` with `success: false` if the user declined
    /// confirmation.
    pub async fn execute(
        &self,
        name: &str,
        params: serde_json::Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        let tool = self.tools.get(name)?;
        if !self.is_enabled(name) {
            return Some(ToolResult::err(format!("工具 {name} 已被禁用")));
        }

        let risk = tool.risk_level();
        if risk >= RiskLevel::Dangerous {
            let request = ConfirmRequest {
                tool_name: name.to_string(),
                action_summary: format!("执行 {} (风险: {:?})", name, risk),
                risk_level: risk,
                params: params.clone(),
            };
            if !ctx.confirm.confirm(&request) {
                return Some(ToolResult::err("用户取消了此操作"));
            }
        }

        Some(tool.execute(params, ctx).await)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "echo".into(),
                description: "Echoes back the input text.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Text to echo" }
                    },
                    "required": ["text"]
                }),
            }
        }

        async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
            ToolResult::ok(text.to_string())
        }
    }

    struct DangerousTool;

    #[async_trait]
    impl Tool for DangerousTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "run_cmd".into(),
                description: "Run a shell command.".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }
        }

        async fn execute(&self, _params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok("executed")
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::Dangerous
        }
    }

    #[tokio::test]
    async fn echo_tool_returns_input() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        let ctx = ToolContext::auto_confirm(".");
        let result = registry
            .execute("echo", json!({"text": "hello"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    #[tokio::test]
    async fn dangerous_tool_blocked_by_deny_all() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DangerousTool));
        let ctx = ToolContext::deny_all(".");
        let result = registry.execute("run_cmd", json!({}), &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("取消"));
    }

    #[test]
    fn remove_unregisters_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        assert!(registry.remove("echo"));
        assert!(registry.all_schemas().is_empty());
        assert!(!registry.remove("echo"));
    }

    #[tokio::test]
    async fn disabled_tool_not_listed_in_enabled_schemas() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        assert_eq!(registry.enabled_schemas().len(), 1);
        registry.set_enabled("echo", false);
        assert_eq!(registry.enabled_schemas().len(), 0);
        assert_eq!(registry.all_schemas().len(), 1); // still listed
    }
}
