//! Fetch a web page and return its text content (HTML tags stripped).
//!
//! Uses `ureq` with `native-tls` (Windows SChannel), the same HTTP stack as
//! `assistant-inference`, so no OpenSSL/ring/C toolchain is required.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;

/// Maximum response body read into memory.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Maximum text returned to the model after extraction.
const MAX_OUTPUT_CHARS: usize = 20_000;
/// HTTP timeout.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Fetch a URL and return the page as plain text.
pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web_fetch".into(),
            description: "获取指定 URL 的网页内容，去除 HTML 标签后返回正文文本（截断到 2 万字符）。仅支持 http/https。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "要获取的网页地址（http 或 https）"}
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) if u.starts_with("http://") || u.starts_with("https://") => u.to_string(),
            Some(_) => return ToolResult::err("仅支持 http:// 或 https:// 开头的 URL"),
            None => return ToolResult::err("缺少 url 参数"),
        };

        // ureq is blocking; run it on the blocking thread pool.
        let result = tokio::task::spawn_blocking(move || fetch_and_extract(&url)).await;
        match result {
            Ok(Ok(text)) => ToolResult::ok(text),
            Ok(Err(e)) => ToolResult::err(format!("获取网页失败: {e}")),
            Err(e) => ToolResult::err(format!("任务执行失败: {e}")),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

fn fetch_and_extract(url: &str) -> Result<String, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS 初始化失败: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout(TIMEOUT)
        .user_agent("LingXi-Agent/0.1 (+https://github.com/3311930677)")
        .build();

    let response = agent
        .get(url)
        .call()
        .map_err(|e| format!("请求失败: {e}"))?;

    let content_type = response.header("content-type").unwrap_or("").to_lowercase();
    let body = response
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let body = if body.len() > MAX_BODY_BYTES {
        body.chars().take(MAX_BODY_BYTES).collect::<String>()
    } else {
        body
    };

    let text = if content_type.contains("html") {
        html_to_text(&body)
    } else {
        // JSON, plain text, markdown, etc. — return as-is.
        body
    };

    if text.trim().is_empty() {
        return Err("页面内容为空".into());
    }
    Ok(truncate_chars(text.trim(), MAX_OUTPUT_CHARS))
}

/// Very small HTML-to-text: drop script/style blocks, strip tags, collapse
/// whitespace. Intentionally naive — this is for LLM consumption, not rendering.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut chars = html.chars().peekable();
    let mut in_tag = false;
    let mut skip_depth = 0u32; // inside <script>/<style>

    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
            }
            continue;
        }
        match c {
            '<' => {
                // Look ahead for tag name to track script/style.
                let rest: String = chars.clone().take(10).collect();
                let lower = rest.to_lowercase();
                // Crude but effective for well-formed pages.
                if lower.starts_with("script") || lower.starts_with("style") {
                    skip_depth += 1;
                } else if lower.starts_with("/script") || lower.starts_with("/style") {
                    skip_depth = skip_depth.saturating_sub(1);
                }
                in_tag = true;
                // Treat block-level boundaries as newlines.
                if lower.starts_with("p")
                    || lower.starts_with("br")
                    || lower.starts_with("div")
                    || lower.starts_with("li")
                    || lower.starts_with("h1")
                    || lower.starts_with("h2")
                    || lower.starts_with("h3")
                    || lower.starts_with("tr")
                {
                    out.push('\n');
                }
            }
            _ if skip_depth > 0 => {}
            _ => out.push(c),
        }
    }

    collapse_whitespace(&out)
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true; // trims leading whitespace too
    let mut newline_run = 0u32;
    for c in text.chars() {
        match c {
            '\n' => {
                newline_run += 1;
                if newline_run <= 2 && !last_was_space {
                    out.push('\n');
                }
                last_was_space = false;
            }
            c if c.is_whitespace() => {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            _ => {
                out.push(c);
                last_was_space = false;
                newline_run = 0;
            }
        }
    }
    out.trim().to_string()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}\n\n...(内容已截断)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_scripts() {
        let html = "<html><head><style>body{color:red}</style></head>\
                    <body><h1>标题</h1><p>第一段</p><script>alert(1)</script>\
                    <p>第二段</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("标题"));
        assert!(text.contains("第一段"));
        assert!(text.contains("第二段"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("color"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn collapses_whitespace() {
        let text = collapse_whitespace("a   b\n\n\n\n\nc");
        assert_eq!(text, "a b\n\nc");
    }

    #[test]
    fn truncation_adds_notice() {
        let long = "字".repeat(30_000);
        let out = truncate_chars(&long, 100);
        assert!(out.contains("内容已截断"));
        assert!(out.chars().count() < 200);
    }
}
