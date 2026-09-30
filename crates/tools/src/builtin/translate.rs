//! Translation tool backed by any OpenAI-compatible chat endpoint.
//!
//! Configuration comes from the process environment (same variables the
//! overlay's cloud settings use), so no secrets are stored in this crate:
//!
//! | Variable                 | Purpose                              |
//! |--------------------------|--------------------------------------|
//! | `LINGXI_OPENAI_API_KEY`  | API key (required)                   |
//! | `LINGXI_OPENAI_BASE_URL` | Endpoint base, default DeepSeek      |
//! | `LINGXI_OPENAI_MODEL`    | Model name, default `deepseek-chat`  |
//!
//! Without `LINGXI_OPENAI_API_KEY` the tool fails with a clear setup message.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-chat";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Translate text between languages using the configured cloud model.
pub struct TranslateTool;

#[async_trait]
impl Tool for TranslateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "translate".into(),
            description: "翻译文本到目标语言。支持中、英、日、韩、法、德等常见语言，通过代码（zh/en/ja...）或中文名指定目标语言。需要配置 LINGXI_OPENAI_API_KEY 环境变量。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "要翻译的文本"},
                    "to": {"type": "string", "description": "目标语言，如 en、zh、ja"}
                },
                "required": ["text", "to"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let text = match params.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolResult::err("缺少有效的 text 参数"),
        };
        let to = match params.get("to").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolResult::err("缺少有效的 to 参数"),
        };
        let from = params
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .trim()
            .to_string();

        let api_key = match std::env::var("LINGXI_OPENAI_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => {
                return ToolResult::err(
                    "翻译功能未配置：请设置 LINGXI_OPENAI_API_KEY 环境变量（\
                     可选：LINGXI_OPENAI_BASE_URL、LINGXI_OPENAI_MODEL）",
                )
            }
        };
        let base_url = std::env::var("LINGXI_OPENAI_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model =
            std::env::var("LINGXI_OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        let job = move || translate_blocking(&api_key, &base_url, &model, &text, &from, &to);
        match tokio::task::spawn_blocking(job).await {
            Ok(Ok(translated)) => ToolResult::ok(translated),
            Ok(Err(e)) => ToolResult::err(format!("翻译失败: {e}")),
            Err(e) => ToolResult::err(format!("任务执行失败: {e}")),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

/// Translate with an explicit provider config. Callers that hold their own
/// settings (e.g. the overlay settings page) should prefer this over the
/// environment-variable path in `TranslateTool::execute`.
#[allow(clippy::too_many_arguments)]
pub fn translate_with_config(
    api_key: &str,
    base_url: &str,
    model: &str,
    text: &str,
    from: &str,
    to: &str,
) -> Result<String, String> {
    translate_blocking(api_key, base_url, model, text, from, to)
}

/// One-shot chat completion with a custom system/user prompt. Callers that
/// need a structured prompt (e.g. batch line translation for the overlay
/// full-screen translate widget) use this instead of `translate_with_config`.
pub fn chat_completion(
    api_key: &str,
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS 初始化失败: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout(TIMEOUT)
        .build();

    let endpoint = chat_completions_url(base_url);
    let payload = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "temperature": 0.1,
        "stream": false
    });

    let response = agent
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let body: String = body.chars().take(300).collect();
                format!("HTTP {code}: {body}")
            }
            other => format!("请求失败: {other}"),
        })?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("解析响应失败: {e}"))?;

    body["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "模型返回了空结果".to_string())
}

/// Normalize an endpoint base (or full URL) into a chat-completions URL.
/// Mirrors the overlay cloud backend: accepts `https://host`,
/// `https://host/v1` and `https://host/v1/chat/completions`.
fn chat_completions_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn translate_blocking(
    api_key: &str,
    base_url: &str,
    model: &str,
    text: &str,
    from: &str,
    to: &str,
) -> Result<String, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS 初始化失败: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout(TIMEOUT)
        .build();

    let endpoint = chat_completions_url(base_url);
    let source_hint = if from.is_empty() || from == "auto" {
        String::new()
    } else {
        format!("The source language is {from}. ")
    };
    let payload = json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You are a translation engine. Output ONLY the translated text, with no explanations, notes, or quotation marks."
            },
            {
                "role": "user",
                "content": format!("{source_hint}Translate the following text to {to}:\n\n{text}")
            }
        ],
        "temperature": 0.1,
        "stream": false
    });

    let response = agent
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let body: String = body.chars().take(300).collect();
                format!("HTTP {code}: {body}")
            }
            other => format!("请求失败: {other}"),
        })?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("解析响应失败: {e}"))?;

    body["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "模型返回了空翻译结果".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_api_key_gives_setup_message() {
        // Ensure the variable is absent for this test.
        std::env::remove_var("LINGXI_OPENAI_API_KEY");
        let tool = TranslateTool;
        let ctx = ToolContext::auto_confirm(".");
        let result = tool
            .execute(json!({"text": "你好", "to": "en"}), &ctx)
            .await;
        assert!(!result.success);
        assert!(result.output.contains("LINGXI_OPENAI_API_KEY"));
    }
}
