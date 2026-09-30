//! Web API 工具（A3-3，差距文档批次 5）：
//!
//! - [`WebSearchTool`]：搜索。配置 `BRAVE_API_KEY` 时走 Brave Search API；
//!   否则回落 DuckDuckGo HTML 端点（免 key）。取代「驱动真浏览器抓 Bing」
//!   ——无需 Playwright 冷启动，延迟从数秒降到亚秒，也不占用浏览器会话。
//! - [`WebFetchTool`]：抓取 URL 正文并抽取为纯文本（去 script/style/标签、
//!   实体解码、截断），供模型读文档/查资料。
//!
//! 安全：**SSRF 防护**——拒绝非 http(s) scheme、localhost/内网域名、以及
//! 解析到回环/私网/链路本地地址的目标（防提示注入把模型变成内网探测跳板）。
//! HTTP 客户端复用 [`crate::gateway::build_model_http_client`]（代理 + NO_PROXY）。

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::OnceLock;

use async_trait::async_trait;
use futures_util::StreamExt;
use regex::Regex;
use serde_json::{json, Value};

use crate::gateway::build_model_http_client;
use crate::tools::{Tool, ToolContext, ToolSpec};

const MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 20_000;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; owo-agent/1.0; +https://github.com/3311930677/LingXi-DesktopAgent)";

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        // 代理 + NO_PROXY（A1-4）与模型通道同口径；失败时退化为默认构建
        //（代理配置错误不应让整个工具集瘫痪）。
        build_model_http_client(10, 30).map(|(client, _)| client).unwrap_or_else(|_| {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP 客户端创建失败")
        })
    })
}

/// SSRF 防护：仅放行解析到公网地址的 http(s) URL。
pub fn is_public_http_url(url: &str) -> Result<(), String> {
    let parsed: reqwest::Url = url
        .parse()
        .map_err(|_| format!("URL 无效：{url}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!("仅支持 http/https，收到 scheme「{}」", parsed.scheme()));
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("URL 缺少主机名".to_string());
    }
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".internal") {
        return Err(format!("拒绝访问内网主机：{host}"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(())
        } else {
            Err(format!("拒绝访问非公网地址：{ip}"))
        };
    }
    // 域名：解析后逐一校验（防 DNS 解析到内网）。
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let lookup = (host.as_str(), port);
    let addrs = lookup
        .to_socket_addrs()
        .map_err(|e| format!("主机解析失败：{host}（{e}）"))?;
    for addr in addrs {
        if !is_public_ip(addr.ip()) {
            return Err(format!(
                "拒绝访问解析到非公网地址的主机：{host} → {}",
                addr.ip()
            ));
        }
    }
    Ok(())
}

/// 公网判定：排除回环/私网/链路本地/CGNAT/ULA 等。
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // 100.64.0.0/10（CGNAT）与 198.18.0.0/15（基准测试）std 未覆盖。
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 198 && (18..=19).contains(&octets[1])))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // ULA fc00::/7
                || (first & 0xffc0) == 0xfe80) // link-local fe80::/10
        }
    }
}

/// 读取响应体，硬上限 [`MAX_BODY_BYTES`]（防超大页面吃内存）。
async fn read_body_limited(response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = stream
        .next()
        .await
        .transpose()
        .map_err(|e| format!("读取响应失败：{e}"))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!("响应超过 {MAX_BODY_BYTES} 字节上限，已中止"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "抓取公开网页并抽取为纯文本（自动去脚本/标签，截断到 max_chars）。仅限公网 http/https（拒绝内网地址）。适合读文档/文章正文；带登录态或 JS 渲染的页面请用 browser_* 工具。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "完整 http/https URL" },
                    "max_chars": { "type": "integer", "description": "返回正文最大字符数（默认 20000）" }
                },
                "required": ["url"]
            }),
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or("缺少 url")?
            .trim()
            .to_string();
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .map(|value| value.min(60_000) as usize)
            .unwrap_or(DEFAULT_MAX_CHARS)
            .max(500);
        is_public_http_url(&url)?;
        let response = http_client()
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| format!("请求失败：{e}"))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if !response.status().is_success() {
            return Err(format!("HTTP {status}：{url}"));
        }
        let body = read_body_limited(response).await?;
        let is_html = content_type.contains("html") || content_type.is_empty();
        let content = if is_html {
            html_to_text(&String::from_utf8_lossy(&body))
        } else {
            // JSON / 纯文本：只做实体解码兜底，不做标签剥离。
            decode_entities(&String::from_utf8_lossy(&body))
        };
        let truncated = content.chars().count() > max_chars;
        let content = content.chars().take(max_chars).collect::<String>();
        Ok(json!({
            "url": url,
            "status": status,
            "content_type": content_type,
            "truncated": truncated,
            "content": if truncated { format!("{content}\n\n[内容已截断，可加大 max_chars 或抓取分页]") } else { content },
        }))
    }
}

/// DuckDuckGo HTML 端点（免 key）；配置 BRAVE_API_KEY 时优先 Brave API。
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "联网搜索，返回标题/链接/摘要列表。默认 DuckDuckGo（免 key）；配置 BRAVE_API_KEY 环境变量时走 Brave API（质量更好）。需要页面正文时对结果 URL 用 web_fetch。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索关键词（建议 2~8 个词，关键名词优先）" },
                    "max_results": { "type": "integer", "description": "结果条数上限（默认 8，最大 15）" }
                },
                "required": ["query"]
            }),
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or("缺少 query")?
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("query 不能为空".to_string());
        }
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|value| value.min(15) as usize)
            .unwrap_or(8)
            .max(1);
        let brave_key = std::env::var("BRAVE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        let (engine, hits) = if let Some(key) = brave_key {
            ("brave".to_string(), brave_search(&query, max_results, key.trim()).await?)
        } else {
            ("duckduckgo".to_string(), ddg_search(&query, max_results).await?)
        };
        if hits.is_empty() {
            return Err(format!(
                "「{query}」无搜索结果。尝试换关键词（少用停用词）或改用 browser_search 工具"
            ));
        }
        let results: Vec<Value> = hits
            .iter()
            .map(|hit| {
                json!({
                    "title": hit.0,
                    "url": hit.1,
                    "snippet": hit.2,
                })
            })
            .collect();
        Ok(json!({
            "query": query,
            "engine": engine,
            "results": results,
            "hint": format!("共 {} 条；读正文请用 web_fetch", results.len()),
        }))
    }
}

/// (title, url, snippet)
type SearchHit = (String, String, String);

async fn ddg_search(query: &str, max_results: usize) -> Result<Vec<SearchHit>, String> {
    let response = http_client()
        .post("https://html.duckduckgo.com/html/")
        .header("User-Agent", USER_AGENT)
        .form(&[("q", query)])
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo 请求失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "DuckDuckGo 返回 HTTP {}（可能限流，稍后重试或配置 BRAVE_API_KEY）",
            response.status()
        ));
    }
    let body = read_body_limited(response).await?;
    Ok(parse_ddg_results(&String::from_utf8_lossy(&body), max_results))
}

async fn brave_search(query: &str, max_results: usize, key: &str) -> Result<Vec<SearchHit>, String> {
    let response = http_client()
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .query(&[("q", query), ("count", &max_results.to_string())])
        .send()
        .await
        .map_err(|e| format!("Brave 请求失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("Brave 返回 HTTP {}", response.status()));
    }
    let body = read_body_limited(response).await?;
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| format!("Brave 响应解析失败：{e}"))?;
    Ok(parse_brave_results(&payload, max_results))
}

/// DuckDuckGo HTML 结果解析（结果链接多为 /l/?uddg=<encoded> 跳转，需还原）。
pub fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchHit> {
    let re_link =
        Regex::new(r#"(?is)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).expect("静态正则");
    let re_snippet =
        Regex::new(r#"(?is)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#).expect("静态正则");
    let re_tag = Regex::new(r"<[^>]+>").expect("静态正则");
    let titles: Vec<(String, String)> = re_link
        .captures_iter(html)
        .filter_map(|caps| {
            let raw_url = caps.get(1)?.as_str();
            let raw_title = caps.get(2)?.as_str();
            let title = decode_entities(&re_tag.replace_all(raw_title, ""));
            Some((
                title.trim().to_string(),
                resolve_ddg_url(&decode_entities(raw_url)),
            ))
        })
        .collect();
    let snippets: Vec<String> = re_snippet
        .captures_iter(html)
        .map(|caps| {
            let raw = caps.get(1).map(|piece| piece.as_str()).unwrap_or("");
            decode_entities(&re_tag.replace_all(raw, "")).trim().to_string()
        })
        .collect();
    titles
        .into_iter()
        .zip(snippets.into_iter().chain(std::iter::repeat(String::new())))
        .filter_map(|((title, url), snippet)| {
            if url.is_empty() {
                None
            } else {
                Some((title, url, snippet))
            }
        })
        .take(max_results)
        .collect()
}

/// DuckDuckGo 跳转链接 `//duckduckgo.com/l/?uddg=<encoded>&rut=...` → 原始 URL。
pub fn resolve_ddg_url(raw: &str) -> String {
    let decoded = decode_entities(raw);
    if let Some(rest) = decoded.split("uddg=").nth(1) {
        let encoded = rest.split('&').next().unwrap_or_default();
        let target = percent_decode(encoded);
        if target.starts_with("http") {
            return target;
        }
    }
    if decoded.starts_with("//") {
        format!("https:{decoded}")
    } else if decoded.starts_with("http") {
        decoded
    } else {
        String::new()
    }
}

pub fn parse_brave_results(payload: &Value, max_results: usize) -> Vec<SearchHit> {
    payload
        .pointer("/web/results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(max_results)
                .filter_map(|item| {
                    let title = item.get("title")?.as_str()?.to_string();
                    let url = item.get("url")?.as_str()?.to_string();
                    let snippet = item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    Some((title, url, snippet))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// HTML → 纯文本：去 script/style/noscript、块级标签转换行、去标签、实体解码、压缩空白。
pub fn html_to_text(html: &str) -> String {
    // Rust regex 不支持反向引用（\1），闭合标签逐个写死。
    let mut text = html.to_string();
    for pattern in [
        r"(?is)<script[^>]*>.*?</script>",
        r"(?is)<style[^>]*>.*?</style>",
        r"(?is)<noscript[^>]*>.*?</noscript>",
        r"(?is)<svg[^>]*>.*?</svg>",
        r"(?is)<iframe[^>]*>.*?</iframe>",
    ] {
        text = Regex::new(pattern)
            .expect("静态正则")
            .replace_all(&text, "")
            .to_string();
    }
    let newline_tags = Regex::new(r"(?is)<(br|/p|/div|/li|/tr|/h[1-6]|/pre|/section|/article|/table)[^>]*>").expect("静态正则");
    let any_tag = Regex::new(r"<[^>]+>").expect("静态正则");
    let blank_lines = Regex::new(r"\n{3,}").expect("静态正则");
    let inline_space = Regex::new(r"[ \t\r]+").expect("静态正则");
    let stage2 = newline_tags.replace_all(&text, "\n");
    let stage3 = any_tag.replace_all(&stage2, " ");
    let stage4 = decode_entities(&stage3);
    let lines: Vec<String> = stage4
        .lines()
        .map(|line| inline_space.replace_all(line.trim(), " ").to_string())
        .filter(|line| !line.is_empty())
        .collect();
    blank_lines
        .replace_all(&lines.join("\n"), "\n\n")
        .to_string()
}

/// 常见 HTML 实体解码（含数字引用；不引新依赖）。
pub fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let named = Regex::new(r"&(amp|lt|gt|quot|apos|nbsp|#39);").expect("静态正则");
    let numeric = Regex::new(r"&#(\d{1,7});").expect("静态正则");
    let named_replaced = named.replace_all(input, |caps: &regex::Captures| {
        match caps.get(1).map(|m| m.as_str()).unwrap_or_default() {
            "amp" => "&".to_string(),
            "lt" => "<".to_string(),
            "gt" => ">".to_string(),
            "quot" | "apos" | "#39" => "\"".to_string(),
            "nbsp" => " ".to_string(),
            other => format!("&{other};"),
        }
    });
    numeric
        .replace_all(&named_replaced, |caps: &regex::Captures| {
            caps.get(1)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        })
        .to_string()
}

/// 手写 percent-decode（%XX；仅用于 uddg 参数还原，长度有限）。
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 3 <= bytes.len() {
            if let Ok(value) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        if bytes[index] == b'+' {
            out.push(b' ');
            index += 1;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssrf_guard_blocks_private_targets() {
        assert!(is_public_http_url("https://example.com/docs").is_ok());
        assert!(is_public_http_url("http://127.0.0.1:4096/x").is_err());
        assert!(is_public_http_url("http://localhost/admin").is_err());
        assert!(is_public_http_url("http://192.168.1.10/router").is_err());
        assert!(is_public_http_url("http://10.0.0.5/").is_err());
        assert!(is_public_http_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(is_public_http_url("file:///etc/passwd").is_err());
        assert!(is_public_http_url("ftp://example.com").is_err());
        assert!(is_public_http_url("http://myhost.local/").is_err());
    }

    #[test]
    fn html_to_text_strips_scripts_and_tags() {
        let html = r#"<html><head><style>body{color:red}</style><script>alert("x")</script></head>
<body><h1>标题</h1><p>第一段 &amp; 第二段 &lt;标签&gt;</p><div>行1<br>行2</div><script>evil()</script></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("标题"), "{text}");
        assert!(text.contains("第一段 & 第二段 <标签>"), "{text}");
        assert!(text.contains("行1"));
        assert!(text.contains("行2"));
        assert!(!text.contains("alert"), "script 内容应被移除：{text}");
        assert!(!text.contains("color:red"), "style 内容应被移除：{text}");
        assert!(!text.contains("evil"), "{text}");
        // 解码后的 &lt;标签&gt; 会合法出现 "<"，只断言无 HTML 标签结构。
        assert!(!text.contains("</"), "不应残留闭合标签：{text}");
        assert!(!Regex::new(r"<[a-z/][a-z0-9]*").unwrap().is_match(&text), "不应残留标签：{text}");
    }

    #[test]
    fn ddg_parse_extracts_titles_urls_and_resolves_redirects() {
        let html = r##"
<div class="result">
  <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs&amp;rut=abc">Example <b>Docs</b></a>
  <a class="result__snippet" href="#">The <em>docs</em> page &amp; more</a>
</div>
<div class="result">
  <a rel="nofollow" class="result__a" href="https://direct.example.org/">直链</a>
  <a class="result__snippet" href="#">摘要二</a>
</div>"##;
        let hits = parse_ddg_results(html, 8);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(
            hits[0].1, "https://example.com/docs",
            "uddg 应还原为原始 URL，实际 {hits:?}"
        );
        assert_eq!(hits[0].0, "Example Docs", "标题应去内联标签");
        assert!(hits[0].2.contains("The docs page & more"));
        assert_eq!(hits[1].1, "https://direct.example.org/");
    }

    #[test]
    fn brave_parse_maps_results() {
        let payload = json!({
            "web": { "results": [
                { "title": "Rust doc", "url": "https://doc.rust-lang.org/", "description": "官方文档" },
                { "title": "无 URL 条目" }
            ]}
        });
        let hits = parse_brave_results(&payload, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Rust doc");
        assert_eq!(hits[0].2, "官方文档");
    }

    #[test]
    fn percent_decode_handles_utf8_and_plus() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(
            percent_decode("%E4%B8%AD%E6%96%87"),
            "中文"
        );
        assert_eq!(percent_decode("100%"), "100%");
    }
}
