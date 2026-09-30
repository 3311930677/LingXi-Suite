//! Key-free web search using DuckDuckGo's HTML endpoint.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use std::time::Duration;

const MAX_QUERY_CHARS: usize = 500;
const MAX_RESULTS: usize = 10;
const MAX_OUTPUT_CHARS: usize = 12_000;

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web_search".into(),
            description: "搜索网页并返回标题、链接和摘要。无需 API 密钥，结果来自 DuckDuckGo。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 10}
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => return ToolResult::err("缺少有效的 query 参数"),
        };
        if query.chars().count() > MAX_QUERY_CHARS {
            return ToolResult::err(format!("query 最多支持 {MAX_QUERY_CHARS} 个字符"));
        }
        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).clamp(1, MAX_RESULTS))
            .unwrap_or(5);

        let result = tokio::task::spawn_blocking(move || search(&query, max_results)).await;
        match result {
            Ok(Ok(output)) => ToolResult::ok(output),
            Ok(Err(error)) => ToolResult::err(format!("搜索失败: {error}")),
            Err(error) => ToolResult::err(format!("搜索任务失败: {error}")),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

fn search(query: &str, max_results: usize) -> Result<String, String> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode(query));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("LingXi-Agent/0.1")
        .build();
    let html = agent
        .get(&url)
        .call()
        .map_err(|error| format!("请求失败: {error}"))?
        .into_string()
        .map_err(|error| format!("读取响应失败: {error}"))?;

    let mut results = Vec::new();
    for block in html.split("result__body").skip(1) {
        let link = extract_attr(block, "result__a", "href");
        let title = extract_tag_text(block, "result__a");
        let snippet = extract_tag_text(block, "result__snippet");
        if let (Some(link), Some(title)) = (link, title) {
            results.push(format!(
                "{}. {}\n链接: {}\n摘要: {}",
                results.len() + 1,
                title,
                link,
                snippet.unwrap_or_default()
            ));
            if results.len() >= max_results {
                break;
            }
        }
    }
    if results.is_empty() {
        return Err("没有找到结果，或搜索服务返回了验证页面".into());
    }
    let output = format!(
        "搜索结果（{} 条）：\n\n{}",
        results.len(),
        results.join("\n\n")
    );
    Ok(output.chars().take(MAX_OUTPUT_CHARS).collect())
}

fn url_encode(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        out
    })
}

fn extract_attr(block: &str, class: &str, attr: &str) -> Option<String> {
    let start = block.find(&format!("class=\"{class}"))?;
    let fragment = &block[start..];
    let value_start = fragment.find(&format!("{attr}=\""))? + attr.len() + 2;
    let value_end = fragment[value_start..].find('"')?;
    Some(decode_entities(
        &fragment[value_start..value_start + value_end],
    ))
}

fn extract_tag_text(block: &str, class: &str) -> Option<String> {
    let start = block.find(&format!("class=\"{class}"))?;
    let fragment = &block[start..];
    let content_start = fragment.find('>')? + 1;
    let content_end = fragment[content_start..]
        .find("</a>")
        .or_else(|| fragment[content_start..].find("</span>"))?;
    let text = strip_tags(&fragment[content_start..content_start + content_end]);
    let text = decode_entities(text.trim());
    (!text.is_empty()).then_some(text)
}

fn strip_tags(value: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for c in value.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(c),
            _ => {}
        }
    }
    output
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query() {
        assert_eq!(url_encode("rust 中文"), "rust%20%E4%B8%AD%E6%96%87");
    }

    #[test]
    fn extracts_result_fields() {
        let html = r#"<a class="result__a" href="https://example.com?a=1&amp;b=2">Title</a><a class="result__snippet">A <b>snippet</b>.</a>"#;
        assert_eq!(
            extract_attr(html, "result__a", "href").as_deref(),
            Some("https://example.com?a=1&b=2")
        );
        assert_eq!(
            extract_tag_text(html, "result__a").as_deref(),
            Some("Title")
        );
    }
}
