//! Cloud-based agent backend using OpenAI-compatible function calling.
//!
//! This module implements [`lingxi_agent::AgentBackend`] on top of an
//! OpenAI-compatible chat-completions endpoint. When the model returns
//! `tool_calls`, the backend extracts the first call and returns
//! [`AgentAction::CallTool`]; otherwise it returns the text content as
//! [`AgentAction::Reply`].

use crate::chat_completions_url;
use crate::CloudConfig;
use anyhow::{Context, Result};
use async_trait::async_trait;
use lingxi_agent::action::AgentAction;
use lingxi_agent::backend::AgentBackend;
use lingxi_agent::error::AgentError;
use lingxi_agent::session::Message;
use lingxi_tools::ToolSchema;
use serde_json::json;
use std::sync::Arc;

/// An agent backend that calls an OpenAI-compatible chat-completions API
/// with function-calling support.
pub struct CloudAgentBackend {
    config: CloudConfig,
}

impl CloudAgentBackend {
    pub fn new(config: CloudConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl AgentBackend for CloudAgentBackend {
    async fn step(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<AgentAction, AgentError> {
        let payload = build_payload(&self.config, messages, tools);

        // Run the blocking HTTP request on a separate thread.
        let config = self.config.clone();
        let result = tokio::task::spawn_blocking(move || post_chat(&config, &payload))
            .await
            .map_err(|e| AgentError::Backend(format!("task join error: {e}")))?
            .map_err(|e| AgentError::Backend(e.to_string()))?;

        parse_response(&result)
    }
}

/// Build the chat-completions request payload with tools and messages.
fn build_payload(
    config: &CloudConfig,
    messages: &[Message],
    tools: &[ToolSchema],
) -> serde_json::Value {
    let messages_json: Vec<serde_json::Value> =
        messages.iter().map(Message::to_openai_json).collect();

    let tools_json: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect();

    json!({
        "model": config.model,
        "messages": messages_json,
        "tools": tools_json,
        "tool_choice": "auto",
        "stream": false,
        "max_tokens": 2048,
    })
}

/// Send the chat-completions request and return the raw response body.
fn post_chat(config: &CloudConfig, payload: &serde_json::Value) -> Result<serde_json::Value> {
    let url = chat_completions_url(&config.endpoint);
    let connector =
        native_tls::TlsConnector::new().context("build cloud native-tls connector")?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(connector))
        .timeout(std::time::Duration::from_secs(90))
        .build();

    let response = agent
        .post(&url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string())
        .with_context(|| format!("POST chat-completions endpoint {url}"))?;

    let body: serde_json::Value =
        serde_json::from_reader(response.into_reader()).context("decode chat-completions response")?;

    Ok(body)
}

/// Parse the API response into an [`AgentAction`].
fn parse_response(body: &serde_json::Value) -> Result<AgentAction, AgentError> {
    let choice = body
        .pointer("/choices/0/message")
        .ok_or_else(|| AgentError::Backend("response has no choices[0].message".into()))?;

    // Check for tool_calls (function calling).
    if let Some(tool_calls) = choice.get("tool_calls").and_then(|v| v.as_array()) {
        if let Some(first_call) = tool_calls.first() {
            let id = first_call
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AgentError::Backend("tool_call missing id".into()))?
                .to_string();

            let name = first_call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AgentError::Backend("tool_call missing function.name".into()))?
                .to_string();

            let arguments_str = first_call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");

            let arguments: serde_json::Value = serde_json::from_str(arguments_str)
                .unwrap_or(serde_json::Value::Null);

            let thought = choice
                .get("content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string());

            return Ok(AgentAction::CallTool {
                id,
                name,
                arguments,
                thought,
            });
        }
    }

    // No tool calls: return the text content as a reply.
    let text = choice
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if text.trim().is_empty() {
        return Err(AgentError::Backend("response content is empty".into()));
    }

    Ok(AgentAction::Reply(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_call_from_response() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "让我先读取选区",
                    "tool_calls": [{
                        "id": "call_001",
                        "type": "function",
                        "function": {
                            "name": "read_selection",
                            "arguments": "{\"mode\": \"full\"}"
                        }
                    }]
                }
            }]
        });

        let action = parse_response(&body).unwrap();
        match action {
            AgentAction::CallTool {
                id,
                name,
                arguments,
                thought,
            } => {
                assert_eq!(id, "call_001");
                assert_eq!(name, "read_selection");
                assert_eq!(arguments["mode"], "full");
                assert_eq!(thought.as_deref(), Some("让我先读取选区"));
            }
            _ => panic!("expected CallTool"),
        }
    }

    #[test]
    fn parse_text_reply_from_response() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "你好，我是灵犀助手。"
                }
            }]
        });

        let action = parse_response(&body).unwrap();
        match action {
            AgentAction::Reply(text) => assert_eq!(text, "你好，我是灵犀助手。"),
            _ => panic!("expected Reply"),
        }
    }

    #[test]
    fn parse_empty_content_errors() {
        let body = json!({
            "choices": [{
                "message": { "content": "" }
            }]
        });
        assert!(parse_response(&body).is_err());
    }

    #[test]
    fn parse_missing_choices_errors() {
        let body = json!({ "error": "rate limit" });
        assert!(parse_response(&body).is_err());
    }
}
