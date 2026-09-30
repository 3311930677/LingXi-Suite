use crate::tools::ToolSpec;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// 图片输入单元（A1-2 多模态）：URL（http/https）或 base64 data URL。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageImage {
    pub url: String,
}

impl MessageImage {
    pub fn from_url(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 图片输入（A1-2）：content 保持纯文本，provider 层在 images 非空且角色
    /// 为 user 时把 wire 内容转成 parts 数组（OpenAI: image_url / Anthropic: image）。
    /// 附加可选字段：老会话记录缺省时视为空，向前兼容。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<MessageImage>,
}

impl ChatMessage {
    pub fn system(content: String) -> Self {
        Self {
            role: "system".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn user(content: String) -> Self {
        Self {
            role: "user".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    /// 带图片的用户消息（A1-2 多模态：截图/贴图进主对话上下文）。
    pub fn user_with_images(content: String, images: Vec<MessageImage>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images,
        }
    }

    pub fn assistant_text(content: String) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn tool(tool_call_id: String, content: String) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(other.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    /// 回合增量 = 当前快照 − 回合前快照（saturating）。
    pub fn saturating_sub(&self, other: &TokenUsage) -> TokenUsage {
        TokenUsage {
            prompt_tokens: self.prompt_tokens.saturating_sub(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_sub(other.completion_tokens),
            total_tokens: self.total_tokens.saturating_sub(other.total_tokens),
        }
    }

    /// 成本估算（美元）：价格按每百万 token 计，默认 0（未知价格不估算）。
    pub fn cost_estimate_usd(&self, input_per_mtok: f64, output_per_mtok: f64) -> f64 {
        self.prompt_tokens as f64 / 1_000_000.0 * input_per_mtok
            + self.completion_tokens as f64 / 1_000_000.0 * output_per_mtok
    }
}

/// 用量预算熔断：返回超限原因；未配置预算时返回 None。
///
/// 累计 token 上限（`OWO_USAGE_TOKEN_BUDGET`）与累计成本上限（美元，
/// `OWO_USAGE_COST_BUDGET_USD`，需配合单价环境变量）任一超限即熔断。
pub fn budget_violation(
    usage: &TokenUsage,
    total_tokens_cap: Option<u64>,
    cost_cap_usd: Option<f64>,
    input_price_per_mtok: f64,
    output_price_per_mtok: f64,
) -> Option<String> {
    if let Some(cap) = total_tokens_cap {
        if usage.total_tokens >= cap {
            return Some(format!(
                "模型用量预算已超限：累计 {} tokens ≥ 上限 {}",
                usage.total_tokens, cap
            ));
        }
    }
    if let Some(cap) = cost_cap_usd {
        let cost = usage.cost_estimate_usd(input_price_per_mtok, output_price_per_mtok);
        if cost >= cap {
            return Some(format!(
                "模型成本预算已超限：累计 ${cost:.6} ≥ 上限 ${cap:.6}"
            ));
        }
    }
    None
}

/// 从模型响应 usage 字段提取 token 用量（兼容 OpenAI/DeepSeek 与 Ollama 字段）。
pub fn parse_usage_value(usage: &Value) -> TokenUsage {
    if !usage.is_object() {
        return TokenUsage::default();
    }
    let prompt = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_eval_count").and_then(Value::as_u64))
        .unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("eval_count").and_then(Value::as_u64))
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt.saturating_add(completion));
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelOutput {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// 流式增量块：正文（对用户可见的回答）或思考（深度思考过程，不写入对话历史）。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Content(String),
    Reasoning(String),
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String>;

    /// 流式补全：文本增量经 `on_delta` 回调；返回最终输出。
    /// 默认实现退化为非流式。
    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        let output = self.complete(messages, tools).await?;
        if let ModelOutput::Text(text) = &output {
            on_delta(text.clone());
        }
        Ok(output)
    }

    /// 带思考通道的流式补全：正文与思考增量统一经 `on_chunk` 回调（类型区分）。
    /// 默认实现委托 `complete_stream`（不支持的 provider 自动兼容，思考块缺失）。
    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        let mut forward = |text: String| on_chunk(StreamChunk::Content(text));
        self.complete_stream(messages, tools, &mut forward).await
    }

    /// 累计 token 用量快照（供回合增量统计；未实现的 Provider 返回零）。
    fn usage_snapshot(&self) -> TokenUsage {
        TokenUsage::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiCompatibleConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 数据出境开关：false 时拒绝一切云端模型调用。
    pub cloud_enabled: bool,
}

/// 设置页保存的接入配置（进程内生效，落盘在 settings.json + 加密信封）。
/// 放在 from_env() 之前裁决，这样已保存的端点/密钥/模型优于启动终端的环境变量，
/// 且无需改动各处 `OpenAiCompatibleConfig::from_env()` 调用点。
#[derive(Debug, Clone, Default)]
struct ProviderOverride {
    base_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
}

static PROVIDER_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<ProviderOverride>>> =
    std::sync::OnceLock::new();

fn provider_override() -> Option<ProviderOverride> {
    PROVIDER_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| (*guard).clone())
}

/// 未显式配置模型时的内置默认（CLI / 服务端 / 网关共用，避免多处漂移）。
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 服务端加载/保存 settings 后调用：让设置页保存的接入配置优先于环境变量。
/// 传 None 的字段表示「该项未配置，回退环境变量」。
pub fn set_provider_override(
    base_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
) {
    let cell = PROVIDER_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = cell.lock() {
        *slot = Some(ProviderOverride {
            base_url,
            api_key,
            model,
        });
    }
}

/// 显式模型覆盖（CLI `--model`）：单独存放，优先级最高（压过设置页保存值与环境变量）。
///
/// 不复用 `PROVIDER_OVERRIDE.model` 的原因：CLI 的 `build_agent` 会在构造 agent 时再调一次
/// `apply_provider_override()`（用 settings 的 model 整体覆盖），把它冲掉。分开放就不依赖调用顺序。
static MODEL_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// 强制当前进程使用指定模型（CLI `--model` 用；此前只写进会话记录，wire 上仍是 env/settings 的值）。
pub fn set_model_override(model: impl Into<String>) {
    let cell = MODEL_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = cell.lock() {
        *slot = Some(model.into());
    }
}

fn model_override() -> Option<String> {
    MODEL_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

#[cfg(test)]
fn clear_model_override() {
    if let Ok(mut slot) = MODEL_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *slot = None;
    }
}

/// 当前进程的模型接入是否就绪（供设置页与首屏判断是否需要引导用户配置）。
pub fn provider_ready() -> bool {
    if wants_anthropic() {
        return crate::anthropic::AnthropicConfig::from_env().is_ok();
    }
    OpenAiCompatibleConfig::from_env().is_ok()
}

impl OpenAiCompatibleConfig {
    pub fn from_env() -> Result<Self, String> {
        let over = provider_override();
        // 已保存值优先，留空则回退环境变量，再回退内置默认。
        let pick = |env_key: &str, saved: Option<&String>| -> Option<String> {
            saved
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .or_else(|| {
                    std::env::var(env_key)
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                })
        };
        let base_url = pick(
            "OPENAI_BASE_URL",
            over.as_ref().and_then(|o| o.base_url.as_ref()),
        )
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let api_key = pick(
            "OPENAI_API_KEY",
            over.as_ref().and_then(|o| o.api_key.as_ref()),
        );
        // 优先级：CLI `--model` 显式覆盖 > 设置页保存值 > OPENAI_MODEL 环境变量 > 内置默认。
        let model = model_override()
            .or_else(|| pick("OPENAI_MODEL", over.as_ref().and_then(|o| o.model.as_ref())))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let api_key = match api_key {
            Some(value) => value,
            None if is_local_endpoint(&base_url) => String::new(),
            None => {
                return Err(
                    "尚未连接模型服务：请在设置页「配置」填写端点与 API 密钥（或设置 OPENAI_BASE_URL 指向本地兼容端点）"
                        .to_string(),
                )
            }
        };
        let cloud_enabled = std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true);
        Ok(Self {
            base_url,
            api_key,
            model,
            cloud_enabled,
        })
    }
}

/// 延迟解析的模型 provider：每次调用前重读接入配置（已保存值 > 环境变量 > 默认）。
///
/// 存在的理由有两条：
/// 1. **首启门不能把服务卡死**——未配置时服务仍要能起来，让设置页可访问、可填写；
/// 2. **保存后即时生效**——设置页写入配置后，下一个回合就用到新端点/密钥，无需重启。
///
/// 配置未就绪时返回可读的错误（由前端首启门先行拦截，正常路径走不到这里）。
///
/// **provider 选择（A1-1）**：`OWO_PROVIDER=anthropic` 且 `ANTHROPIC_API_KEY`
/// 可用 → Anthropic 原生（prompt caching / 原生 tool_use / 多模态 image 块）；
/// 其余情况 → OpenAI-compatible（默认，覆盖 DeepSeek/Ollama/多数代理）。
pub struct DeferredProvider {
    cached: std::sync::Mutex<Option<(String, Arc<dyn ModelProvider>)>>,
}

impl Default for DeferredProvider {
    fn default() -> Self {
        Self {
            cached: std::sync::Mutex::new(None),
        }
    }
}

impl DeferredProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取当前配置对应的 provider；配置指纹变化则重建（保存设置后自动换新）。
    fn resolve(&self) -> Result<Arc<dyn ModelProvider>, String> {
        // 指纹 = provider 种类 + 配置摘要：种类或端点/密钥/模型任一变化即重建。
        let resolved: (String, Arc<dyn ModelProvider>) = if wants_anthropic() {
            let config = crate::anthropic::AnthropicConfig::from_env()?;
            let fingerprint = format!(
                "anthropic|{}|{}|{}|{}",
                config.base_url, config.api_key, config.model, config.cloud_enabled
            );
            let provider: Arc<dyn ModelProvider> =
                Arc::new(crate::anthropic::AnthropicProvider::new(config)?);
            (fingerprint, provider)
        } else {
            let config = OpenAiCompatibleConfig::from_env()?;
            let fingerprint = format!(
                "openai|{}|{}|{}|{}",
                config.base_url, config.api_key, config.model, config.cloud_enabled
            );
            let provider: Arc<dyn ModelProvider> =
                Arc::new(OpenAiCompatibleProvider::new(config.clone())?);
            (fingerprint, provider)
        };
        let mut slot = self
            .cached
            .lock()
            .map_err(|_| "provider 缓存锁中毒".to_string())?;
        if let Some((cached_fingerprint, provider)) = slot.as_ref() {
            if *cached_fingerprint == resolved.0 {
                return Ok(Arc::clone(provider));
            }
        }
        *slot = Some((resolved.0.clone(), Arc::clone(&resolved.1)));
        Ok(resolved.1)
    }
}

/// 是否要求 Anthropic 原生通道（`OWO_PROVIDER=anthropic`，大小写不敏感）。
fn wants_anthropic() -> bool {
    std::env::var("OWO_PROVIDER")
        .map(|value| value.trim().eq_ignore_ascii_case("anthropic"))
        .unwrap_or(false)
}

#[async_trait::async_trait]
impl ModelProvider for DeferredProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.resolve()?.complete(messages, tools).await
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        self.resolve()?
            .complete_stream(messages, tools, on_delta)
            .await
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.resolve()?
            .complete_stream_with_reasoning(messages, tools, on_chunk)
            .await
    }

    /// 转发真实 provider 的用量累计（否则外层 ResilientProvider 聚合到零值，
    /// 回合汇报卡的 token 消耗会一直缺失）。
    fn usage_snapshot(&self) -> TokenUsage {
        self.resolve()
            .map(|provider| provider.usage_snapshot())
            .unwrap_or_default()
    }
}

/// OpenAI-compatible `/chat/completions` 客户端（覆盖 OpenAI、DeepSeek、Ollama、多数代理）。
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    direct_client: Option<reqwest::Client>,
    config: OpenAiCompatibleConfig,
    usage: std::sync::Mutex<TokenUsage>,
}

/// 构建模型 HTTP 客户端：代理（OWO_HTTP_PROXY/HTTPS_PROXY/HTTP_PROXY）+
/// NO_PROXY 排除列表（A1-4：127.0.0.1/localhost 等本地端点必须直连，
/// 否则配置了 OWO_HTTP_PROXY 后本地兼容端点流量也会被推进代理）。
pub(crate) fn build_model_http_client(
    connect_timeout_secs: u64,
    total_timeout_secs: u64,
) -> Result<(reqwest::Client, bool), String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(connect_timeout_secs))
        .timeout(std::time::Duration::from_secs(total_timeout_secs));
    let mut has_proxy = false;
    for name in [
        "OWO_HTTP_PROXY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "https_proxy",
        "http_proxy",
    ] {
        if let Ok(proxy) = std::env::var(name) {
            if !proxy.trim().is_empty() {
                let mut proxy = reqwest::Proxy::all(proxy)
                    .map_err(|e| format!("代理配置无效（{name}）：{e}"))?;
                has_proxy = true;
                let no_proxy = std::env::var("NO_PROXY")
                    .or_else(|_| std::env::var("no_proxy"))
                    .unwrap_or_default();
                if !no_proxy.trim().is_empty() {
                    if let Some(exclusions) = reqwest::NoProxy::from_string(&no_proxy) {
                        proxy = proxy.no_proxy(Some(exclusions));
                    }
                }
                builder = builder.proxy(proxy);
                break;
            }
        }
    }
    let client = builder
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败：{e}"))?;
    Ok((client, has_proxy))
}

impl OpenAiCompatibleProvider {
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, String> {
        let (client, has_proxy) = build_model_http_client(10, 180)?;
        let direct_client = if has_proxy {
            Some(
                reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .map_err(|e| format!("直连 HTTP 客户端创建失败：{e}"))?,
            )
        } else {
            None
        };
        Ok(Self {
            client,
            direct_client,
            config,
            usage: std::sync::Mutex::new(TokenUsage::default()),
        })
    }

    fn record_usage(&self, usage: &Value) {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens == 0 && parsed.prompt_tokens == 0 && parsed.completion_tokens == 0 {
            return;
        }
        if let Ok(mut current) = self.usage.lock() {
            current.add(&parsed);
        }
    }

    /// 读取环境变量预算并检查当前累计用量是否超限。
    fn usage_budget_check(&self) -> Option<String> {
        let total_cap = std::env::var("OWO_USAGE_TOKEN_BUDGET")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        let cost_cap = std::env::var("OWO_USAGE_COST_BUDGET_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok());
        if total_cap.is_none() && cost_cap.is_none() {
            return None;
        }
        let input_price = std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let output_price = std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let usage = self.usage.lock().map(|usage| *usage).unwrap_or_default();
        budget_violation(&usage, total_cap, cost_cap, input_price, output_price)
    }

    /// 发送请求：优先代理客户端，失败自动切直连重试一次（多轮流式挂起时稳定）。
    async fn post_chat(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        let mut last_error = String::new();
        let attempts: Vec<(&str, &reqwest::Client)> = {
            let mut list = vec![("proxy", &self.client)];
            if let Some(direct) = &self.direct_client {
                list.push(("direct", direct));
            }
            list
        };
        for (label, client) in attempts {
            let request = client
                .post(url)
                .json(body)
                .timeout(std::time::Duration::from_secs(120));
            let request = if self.config.api_key.is_empty() {
                request
            } else {
                request.bearer_auth(&self.config.api_key)
            };
            match request.send().await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let text = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "无响应体".to_string());
                    return Err(format!("模型返回 {status}：{text}"));
                }
                Err(error) => {
                    last_error = format!("{label}: {error}");
                }
            }
        }
        Err(format!("模型请求失败：{last_error}"))
    }

    /// 数据出境开关：优先读运行时环境变量（支持设置页即时切换），缺省用启动配置。
    fn cloud_enabled(&self) -> bool {
        if is_local_endpoint(&self.config.base_url) {
            return true;
        }
        std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(self.config.cloud_enabled)
    }

    /// 当前模型：用构造时的配置（`from_env` 已按「设置页保存值 > OPENAI_MODEL > 默认」解析好）。
    ///
    /// 这里不再直读环境变量：此前 env 优先级高于配置，导致「设置页选了模型但进程里存在
    /// OPENAI_MODEL 时永远不生效」（桌面壳还会注入 OPENAI_MODEL=local，症状相同）。
    /// 热切换依旧成立——`DeferredProvider` 每次调用前重读 `from_env()` 并重建 provider。
    fn model(&self) -> String {
        self.config.model.clone()
    }

    /// 推理档位（`reasoning_effort`）：读运行时环境变量（设置页保存后即时生效）。
    /// 只认 minimal/low/medium/high；未设置或取值非法则返回 None = 不发送该参数，
    /// 避免不支持它的 OpenAI 兼容端点因为未知字段直接 400。
    fn reasoning_effort(&self) -> Option<String> {
        let value = std::env::var("OWO_REASONING_EFFORT")
            .ok()?
            .trim()
            .to_ascii_lowercase();
        if matches!(value.as_str(), "minimal" | "low" | "medium" | "high") {
            Some(value)
        } else {
            None
        }
    }

    fn request_body(&self, messages: &[ChatMessage], tools: &[ToolSpec], stream: bool) -> Value {
        let tool_payload: Vec<Value> = tools
            .iter()
            .map(|spec| {
                json!({
                    "type": "function",
                    "function": {
                        "name": spec.name,
                        "description": spec.description,
                        "parameters": spec.input_schema,
                    }
                })
            })
            .collect();
        let messages_payload: Vec<Value> = messages
            .iter()
            .map(|message| {
                // A1-2 多模态：user 消息带图片时 content 升级为 parts 数组
                //（其余角色仍为字符串——OpenAI 兼容端点对 tool/system 的
                // 数组 content 支持不一，图片统一从 user 通道进）。
                let content_value = if message.role == "user" && !message.images.is_empty() {
                    let mut parts: Vec<Value> = Vec::new();
                    if let Some(text) = message.content.as_deref().filter(|t| !t.is_empty()) {
                        parts.push(json!({ "type": "text", "text": text }));
                    }
                    for image in &message.images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": { "url": image.url },
                        }));
                    }
                    Value::Array(parts)
                } else {
                    json!(message.content)
                };
                let mut wire = json!({
                    "role": message.role,
                    "content": content_value,
                });
                if let Some(tool_call_id) = &message.tool_call_id {
                    wire["tool_call_id"] = Value::String(tool_call_id.clone());
                }
                if let Some(tool_calls) = &message.tool_calls {
                    let wire_calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": serde_json::to_string(&call.arguments)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }
                            })
                        })
                        .collect();
                    wire["tool_calls"] = Value::Array(wire_calls);
                }
                wire
            })
            .collect();

        let mut body = json!({
            "model": self.model(),
            "messages": messages_payload,
            "stream": stream,
        });
        if !tool_payload.is_empty() {
            body["tools"] = Value::Array(tool_payload);
        }
        if stream {
            body["stream_options"] = json!({ "include_usage": true });
        }
        // 推理档位只在用户显式选择时才下发（默认请求体与旧版完全一致）。
        if let Some(effort) = self.reasoning_effort() {
            body["reasoning_effort"] = Value::String(effort);
        }
        body
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct StreamDelta {
    pub content: Option<String>,
    /// 思考增量（DeepSeek `reasoning_content` 等；不写入对话历史）。
    pub reasoning: Option<String>,
    /// 原始 tool_calls 增量片段（JSON 值）。
    pub tool_call_fragments: Vec<Value>,
    /// 末尾 usage 块（OpenAI-compatible 流式响应在最后一条 data 中给出）。
    pub usage: Option<TokenUsage>,
}

/// 解析一条 `data:` 负载。空负载/心跳返回 None。
pub fn parse_sse_payload(payload: &str) -> Option<StreamDelta> {
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    let delta = value.pointer("/choices/0/delta")?;
    let content = delta
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let reasoning = delta
        .get("reasoning_content")
        .or_else(|| delta.get("reasoning"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let tool_call_fragments = delta
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let usage = value
        .get("usage")
        .map(parse_usage_value)
        .filter(|usage| usage.total_tokens > 0 || usage.prompt_tokens > 0);
    if content.is_none() && reasoning.is_none() && tool_call_fragments.is_empty() && usage.is_none()
    {
        return None;
    }
    Some(StreamDelta {
        content,
        reasoning,
        tool_call_fragments,
        usage,
    })
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

fn accumulate_tool_fragments(
    accumulators: &mut HashMap<usize, ToolCallAccumulator>,
    fragments: &[Value],
) {
    for fragment in fragments {
        let Some(index) = fragment.get("index").and_then(Value::as_u64) else {
            continue;
        };
        let index = index as usize;
        let entry = accumulators.entry(index).or_default();
        if let Some(id) = fragment.get("id").and_then(Value::as_str) {
            entry.id = id.to_string();
        }
        if let Some(name) = fragment.pointer("/function/name").and_then(Value::as_str) {
            entry.name = name.to_string();
        }
        if let Some(arguments) = fragment
            .pointer("/function/arguments")
            .and_then(Value::as_str)
        {
            entry.arguments.push_str(arguments);
        }
    }
}

fn build_tool_calls(
    accumulators: &mut HashMap<usize, ToolCallAccumulator>,
) -> Option<Vec<ToolCall>> {
    if accumulators.is_empty() {
        return None;
    }
    let mut calls: Vec<(usize, ToolCall)> = accumulators
        .drain()
        .map(|(index, accum)| {
            (
                index,
                ToolCall {
                    id: if accum.id.is_empty() {
                        format!("call_{index}")
                    } else {
                        accum.id
                    },
                    name: accum.name,
                    arguments: serde_json::from_str(&accum.arguments).unwrap_or(Value::Null),
                },
            )
        })
        .collect();
    calls.sort_by_key(|(index, _)| *index);
    Some(calls.into_iter().map(|(_, call)| call).collect())
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        if let Some(reason) = self.usage_budget_check() {
            return Err(reason);
        }
        let body = self.request_body(messages, tools, false);
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self.post_chat(&url, &body).await?;

        let payload: Value = response
            .json()
            .await
            .map_err(|e| format!("模型响应解析失败：{e}"))?;
        self.record_usage(payload.get("usage").unwrap_or(&Value::Null));
        let message = payload
            .pointer("/choices/0/message")
            .ok_or_else(|| "响应缺少 choices[0].message".to_string())?;
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| {
                        let id = call.get("id")?.as_str()?.to_string();
                        let name = call.pointer("/function/name")?.as_str()?.to_string();
                        let arguments = call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .and_then(|raw| serde_json::from_str(raw).ok())
                            .unwrap_or(Value::Null);
                        Some(ToolCall {
                            id,
                            name,
                            arguments,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|calls: &Vec<ToolCall>| !calls.is_empty());

        if let Some(tool_calls) = tool_calls {
            Ok(ModelOutput::ToolCalls(tool_calls))
        } else if let Some(content) = content {
            Ok(ModelOutput::Text(content))
        } else {
            Err("模型响应既无文本也无工具调用".to_string())
        }
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        // 适配器：只转发正文块（思考块被忽略）；真实实现见 complete_stream_with_reasoning。
        let mut forward = |chunk: StreamChunk| {
            if let StreamChunk::Content(text) = chunk {
                on_delta(text);
            }
        };
        self.complete_stream_with_reasoning(messages, tools, &mut forward)
            .await
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        if let Some(reason) = self.usage_budget_check() {
            return Err(reason);
        }
        let body = self.request_body(messages, tools, true);
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self.post_chat(&url, &body).await?;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut accumulators: HashMap<usize, ToolCallAccumulator> = HashMap::new();
        let mut utf8_pending = Vec::new();
        let mut saw_sse = false;

        while let Some(chunk) =
            tokio::time::timeout(std::time::Duration::from_secs(60), stream.next())
                .await
                .map_err(|_| "模型流式输出空闲超时（60s 无数据）".to_string())?
        {
            let chunk = chunk.map_err(|e| format!("流式读取失败：{e}"))?;
            append_utf8_chunk(&mut buffer, &mut utf8_pending, &chunk);
            if let Some(usage) = consume_stream_buffer(
                &mut buffer,
                &mut content,
                &mut accumulators,
                on_chunk,
                &mut saw_sse,
            ) {
                self.record_usage(&json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                }));
                // R9：流式路径每块检查预算，超限立即停轮并返回可读错误。
                if let Some(reason) = self.usage_budget_check() {
                    return Err(reason);
                }
            }
        }

        if !utf8_pending.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_pending));
        }
        if !buffer.trim().is_empty() {
            buffer.push('\n');
            if let Some(usage) = consume_stream_buffer(
                &mut buffer,
                &mut content,
                &mut accumulators,
                on_chunk,
                &mut saw_sse,
            ) {
                self.record_usage(&json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                }));
                if let Some(reason) = self.usage_budget_check() {
                    return Err(reason);
                }
            }
        }

        if !saw_sse {
            return Err("模型流式响应为空或不是 SSE 格式".to_string());
        }

        if let Some(tool_calls) = build_tool_calls(&mut accumulators) {
            Ok(ModelOutput::ToolCalls(tool_calls))
        } else {
            Ok(ModelOutput::Text(content))
        }
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.usage.lock().map(|usage| *usage).unwrap_or_default()
    }
}

/// R9 韧性层：重试策略（指数退避 + jitter）。
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// 重试次数（不含首次请求）。
    pub max_retries: usize,
    /// 首次退避基数（毫秒）。
    pub base_delay_ms: u64,
    /// 退避上限（毫秒）。
    pub max_delay_ms: u64,
    /// 429 是否重试。
    pub retry_429: bool,
    /// 连接/超时/空闲看门狗类失败是否重试。
    pub retry_network: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 8_000,
            retry_429: true,
            retry_network: true,
        }
    }
}

impl RetryPolicy {
    /// 环境变量：OWO_MODEL_RETRY_MAX / OWO_MODEL_RETRY_BASE_MS / OWO_MODEL_RETRY_MAX_DELAY_MS。
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            max_retries: std::env::var("OWO_MODEL_RETRY_MAX")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default.max_retries),
            base_delay_ms: std::env::var("OWO_MODEL_RETRY_BASE_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default.base_delay_ms),
            max_delay_ms: std::env::var("OWO_MODEL_RETRY_MAX_DELAY_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(default.max_delay_ms),
            ..default
        }
    }

    /// 第 `attempt` 次重试前延迟：`min(max, base × 2^attempt)` + 0..20% jitter。
    pub fn delay_for(&self, attempt: usize) -> std::time::Duration {
        let exponential = self
            .base_delay_ms
            .saturating_mul(1_u64 << attempt.min(10))
            .min(self.max_delay_ms);
        let jitter = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.subsec_nanos())
                .unwrap_or(0);
            (nanos, attempt).hash(&mut hasher);
            hasher.finish() % 21 // 0..=20
        };
        let delay =
            exponential.saturating_add(exponential.saturating_mul(jitter).saturating_div(100));
        std::time::Duration::from_millis(delay)
    }
}

/// R9 韧性层：熔断器状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// R9 韧性层：连续失败熔断器（Closed → Open → HalfOpen → Closed）。
pub struct CircuitBreaker {
    failure_threshold: usize,
    cooldown: std::time::Duration,
    consecutive_failures: std::sync::atomic::AtomicUsize,
    state: std::sync::Mutex<BreakerState>,
    opened_at: std::sync::Mutex<Option<std::time::Instant>>,
    half_open_probe: std::sync::atomic::AtomicBool,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(5, std::time::Duration::from_secs(10))
    }
}

impl CircuitBreaker {
    pub fn new(failure_threshold: usize, cooldown: std::time::Duration) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            cooldown,
            consecutive_failures: std::sync::atomic::AtomicUsize::new(0),
            state: std::sync::Mutex::new(BreakerState::Closed),
            opened_at: std::sync::Mutex::new(None),
            half_open_probe: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 环境变量：OWO_MODEL_CIRCUIT_THRESHOLD（默认 5）/ OWO_MODEL_CIRCUIT_COOLDOWN_SECS（默认 10）。
    pub fn from_env() -> Self {
        let default = Self::default();
        let threshold = std::env::var("OWO_MODEL_CIRCUIT_THRESHOLD")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default.failure_threshold);
        let cooldown = std::env::var("OWO_MODEL_CIRCUIT_COOLDOWN_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .unwrap_or(default.cooldown);
        Self::new(threshold, cooldown)
    }

    pub fn state(&self) -> BreakerState {
        match *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
        {
            BreakerState::Open => {
                let opened = *self
                    .opened_at
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if opened.is_some_and(|at| at.elapsed() >= self.cooldown) {
                    BreakerState::HalfOpen
                } else {
                    BreakerState::Open
                }
            }
            other => other,
        }
    }

    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 是否放行请求；HalfOpen 仅放行一个探测请求。
    pub fn allow_request(&self) -> bool {
        match self.state() {
            BreakerState::Closed => true,
            BreakerState::Open => false,
            BreakerState::HalfOpen => self
                .half_open_probe
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok(),
        }
    }

    pub fn record_success(&self) {
        self.consecutive_failures
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.half_open_probe
            .store(false, std::sync::atomic::Ordering::Relaxed);
        *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = BreakerState::Closed;
        *self
            .opened_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    }

    pub fn record_failure(&self) {
        let failures = self
            .consecutive_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if failures >= self.failure_threshold {
            *self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = BreakerState::Open;
            *self
                .opened_at
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(std::time::Instant::now());
            self.half_open_probe
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// 强制复位（运维/测试）。
    pub fn reset(&self) {
        self.record_success();
    }
}

/// 错误是否可重试：网络/5xx/429/空闲看门狗 → 可；预算/出境/解析/4xx → 不可。
fn is_retriable(error: &str, policy: &RetryPolicy) -> bool {
    if error.contains("预算已超限") || error.contains("数据出境") {
        return false;
    }
    if error.contains("模型返回 429") {
        return policy.retry_429;
    }
    if error.contains("模型返回 5") {
        return true;
    }
    if error.contains("模型请求失败")
        || error.contains("流式输出空闲超时")
        || error.contains("流式读取失败")
        || error.contains("连接")
        || error.contains("超时")
    {
        return policy.retry_network;
    }
    false
}

/// R9 韧性层：Provider 链（强模型 → 次选云 → 本地），带指数退避重试与熔断器。
/// failover 语义：primary 连续失败 → 熔断打开 → 快速失败；冷却后 HalfOpen 探测。
pub struct ResilientProvider {
    primary: Arc<dyn ModelProvider>,
    fallbacks: Vec<Arc<dyn ModelProvider>>,
    breaker: Arc<CircuitBreaker>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for ResilientProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResilientProvider")
            .field("fallbacks", &self.fallbacks.len())
            .field("breaker", &self.breaker.state())
            .finish()
    }
}

impl ResilientProvider {
    pub fn new(
        primary: Arc<dyn ModelProvider>,
        fallbacks: Vec<Arc<dyn ModelProvider>>,
        breaker: CircuitBreaker,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            primary,
            fallbacks,
            breaker: Arc::new(breaker),
            retry,
        }
    }

    /// 环境变量构造：主 = OPENAI_BASE_URL/OPENAI_API_KEY/OPENAI_MODEL；
    /// fallback = OWO_MODEL_FALLBACK_BASE_URLS（逗号分隔；本地端点无需 key）。
    pub fn from_env() -> Result<Self, String> {
        let config = OpenAiCompatibleConfig::from_env()?;
        Self::from_config(config)
    }

    /// 以显式主配置构造（CLI 接线用，主配置的 model/api_key 已确定）；
    /// fallback 仍读 OWO_MODEL_FALLBACK_BASE_URLS（同 model；本地端点无需 key）。
    pub fn from_config(config: OpenAiCompatibleConfig) -> Result<Self, String> {
        let primary: Arc<dyn ModelProvider> =
            Arc::new(OpenAiCompatibleProvider::new(config.clone())?);
        Self::from_primary(primary, &config)
    }

    /// 主 provider 延迟解析：每次调用前重读接入配置，因此设置页保存后**下一个回合即生效**，
    /// 无需重启服务。未配置时也不会让启动失败（错误在调用点才暴露，由前端首启门先行拦截）。
    /// fallback 仍读 OWO_MODEL_FALLBACK_BASE_URLS。
    pub fn from_deferred() -> Self {
        // 配置未就绪时用空种子构造 fallback 链；真正的主 provider 是 DeferredProvider。
        let seed = OpenAiCompatibleConfig::from_env().unwrap_or_else(|_| OpenAiCompatibleConfig {
            base_url: String::new(),
            api_key: String::new(),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            cloud_enabled: true,
        });
        Self::from_primary(Arc::new(DeferredProvider::new()), &seed)
            .expect("fallback 链构造不应失败")
    }

    fn from_primary(
        primary: Arc<dyn ModelProvider>,
        config: &OpenAiCompatibleConfig,
    ) -> Result<Self, String> {
        let mut fallbacks: Vec<Arc<dyn ModelProvider>> = Vec::new();
        if let Ok(urls) = std::env::var("OWO_MODEL_FALLBACK_BASE_URLS") {
            for url in urls.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let local = is_local_endpoint(url);
                let fallback_config = OpenAiCompatibleConfig {
                    base_url: url.to_string(),
                    api_key: if local {
                        String::new()
                    } else {
                        config.api_key.clone()
                    },
                    model: config.model.clone(),
                    cloud_enabled: config.cloud_enabled || local,
                };
                fallbacks.push(Arc::new(OpenAiCompatibleProvider::new(fallback_config)?));
            }
        }
        Ok(Self::new(
            primary,
            fallbacks,
            CircuitBreaker::from_env(),
            RetryPolicy::from_env(),
        ))
    }

    pub fn breaker(&self) -> &CircuitBreaker {
        &self.breaker
    }

    pub fn retry(&self) -> &RetryPolicy {
        &self.retry
    }

    fn providers(&self) -> Vec<Arc<dyn ModelProvider>> {
        let mut providers = Vec::with_capacity(1 + self.fallbacks.len());
        providers.push(Arc::clone(&self.primary));
        providers.extend(self.fallbacks.iter().cloned());
        providers
    }

    /// 对单个 provider 执行带退避重试的调用；返回 (结果, 是否命中可重试失败)。
    async fn call_with_retry(
        provider: &Arc<dyn ModelProvider>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        retry: &RetryPolicy,
    ) -> (Result<ModelOutput, String>, bool) {
        let mut attempt = 0;
        loop {
            match provider.complete(messages, tools).await {
                Ok(output) => return (Ok(output), false),
                Err(error) => {
                    let retriable = is_retriable(&error, retry);
                    if !retriable || attempt >= retry.max_retries {
                        return (Err(error), retriable);
                    }
                    tokio::time::sleep(retry.delay_for(attempt)).await;
                    attempt += 1;
                }
            }
        }
    }

    /// 总成本/用量快照：聚合主链与 fallback（各 Provider 自记）。
    fn aggregate_usage(&self) -> TokenUsage {
        let mut total = TokenUsage::default();
        for provider in self.providers() {
            total.add(&provider.usage_snapshot());
        }
        total
    }

    /// 流式调用内核：实时转发增量（用户能立即看到思考/正文），
    /// 尚未产生任何输出时按重试策略重试/降级；一旦已发出增量则不再重试，
    /// 避免重复前缀（如思考流前几个字再次出现）。
    async fn complete_stream_inner(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        if !self.breaker.allow_request() {
            return Err(format!(
                "模型网关熔断器打开（连续失败 {} 次），请稍后重试",
                self.breaker.consecutive_failures()
            ));
        }
        let mut errors: Vec<String> = Vec::new();
        let mut retriable_seen = false;
        for provider in self.providers() {
            let mut attempt = 0;
            let outcome = loop {
                let mut produced = false;
                let mut forward_mut: &mut (dyn FnMut(StreamChunk) + Send) =
                    &mut |chunk: StreamChunk| {
                        produced = true;
                        on_chunk(chunk);
                    };
                let result = provider
                    .complete_stream_with_reasoning(messages, tools, &mut forward_mut)
                    .await;
                match result {
                    Ok(output) => {
                        self.breaker.record_success();
                        return Ok(output);
                    }
                    Err(error) => {
                        let retriable = is_retriable(&error, &self.retry);
                        if produced || !retriable || attempt >= self.retry.max_retries {
                            break (error, retriable);
                        }
                        tokio::time::sleep(self.retry.delay_for(attempt)).await;
                        attempt += 1;
                    }
                }
            };
            errors.push(outcome.0);
            retriable_seen = retriable_seen || outcome.1;
            if !outcome.1 {
                break;
            }
        }
        self.breaker.record_failure();
        Err(format!("模型网关全部失败：{}", errors.join("；")))
    }
}

#[async_trait]
impl ModelProvider for ResilientProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if !self.breaker.allow_request() {
            return Err(format!(
                "模型网关熔断器打开（连续失败 {} 次），请稍后重试",
                self.breaker.consecutive_failures()
            ));
        }
        let mut errors: Vec<String> = Vec::new();
        for provider in self.providers() {
            let (result, retriable) =
                Self::call_with_retry(&provider, messages, tools, &self.retry).await;
            match result {
                Ok(output) => {
                    self.breaker.record_success();
                    return Ok(output);
                }
                Err(error) => {
                    errors.push(error);
                    // 不可重试错误（预算/出境/解析）不降级到下一 Provider。
                    if !retriable {
                        break;
                    }
                }
            }
        }
        self.breaker.record_failure();
        Err(format!("模型网关全部失败：{}", errors.join("；")))
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        // 适配器：只转发正文块（思考块被忽略）。
        let mut forward = |chunk: StreamChunk| {
            if let StreamChunk::Content(text) = chunk {
                on_delta(text);
            }
        };
        self.complete_stream_inner(messages, tools, &mut forward)
            .await
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.complete_stream_inner(messages, tools, on_chunk).await
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.aggregate_usage()
    }
}

fn is_local_endpoint(base_url: &str) -> bool {
    let authority = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn append_utf8_chunk(buffer: &mut String, pending: &mut Vec<u8>, chunk: &[u8]) {
    pending.extend_from_slice(chunk);
    match String::from_utf8(std::mem::take(pending)) {
        Ok(text) => buffer.push_str(&text),
        Err(error) => {
            let bytes = error.into_bytes();
            let valid = std::str::from_utf8(&bytes)
                .map(|_| bytes.len())
                .unwrap_or_else(|error| error.valid_up_to());
            buffer.push_str(std::str::from_utf8(&bytes[..valid]).unwrap_or_default());
            pending.extend_from_slice(&bytes[valid..]);
        }
    }
}

fn consume_stream_buffer(
    buffer: &mut String,
    content: &mut String,
    accumulators: &mut HashMap<usize, ToolCallAccumulator>,
    on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    saw_sse: &mut bool,
) -> Option<TokenUsage> {
    let mut usage = None;
    while let Some(newline) = buffer.find('\n') {
        let line = buffer[..newline].trim().to_string();
        buffer.drain(..=newline);
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        *saw_sse = true;
        if payload.trim() == "[DONE]" {
            continue;
        }
        if let Some(delta) = parse_sse_payload(payload) {
            if delta.usage.is_some() {
                usage = delta.usage;
            }
            if let Some(reasoning) = delta.reasoning {
                on_chunk(StreamChunk::Reasoning(reasoning));
            }
            if let Some(delta_content) = delta.content {
                content.push_str(&delta_content);
                on_chunk(StreamChunk::Content(delta_content));
            }
            accumulate_tool_fragments(accumulators, &delta.tool_call_fragments);
        }
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量依赖的网关测试串行执行，避免并行设置互相干扰。
    static ENV_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[test]
    fn parses_content_delta() {
        let delta = parse_sse_payload(r#"{"choices":[{"delta":{"content":"你好"}}]}"#).unwrap();
        assert_eq!(delta.content.as_deref(), Some("你好"));
        assert!(delta.tool_call_fragments.is_empty());
    }

    #[test]
    fn parses_tool_call_fragments_and_assembles() {
        let delta = parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
        )
        .unwrap();
        assert_eq!(delta.content, None);
        assert_eq!(delta.tool_call_fragments.len(), 1);

        let mut accumulators = HashMap::new();
        accumulate_tool_fragments(&mut accumulators, &delta.tool_call_fragments);
        let delta2 = parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.txt\"}"}}]}}]}"#,
        )
        .unwrap();
        accumulate_tool_fragments(&mut accumulators, &delta2.tool_call_fragments);

        let calls = build_tool_calls(&mut accumulators).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "a.txt");
    }

    #[test]
    fn ignores_heartbeat_and_done() {
        assert!(parse_sse_payload("").is_none());
        assert!(parse_sse_payload("[DONE]").is_none());
        assert!(parse_sse_payload(": keep-alive").is_none());
    }

    #[test]
    fn parses_usage_value_for_openai_and_ollama_fields() {
        let openai = parse_usage_value(&json!({
            "prompt_tokens": 120,
            "completion_tokens": 30,
            "total_tokens": 150,
        }));
        assert_eq!(openai.prompt_tokens, 120);
        assert_eq!(openai.completion_tokens, 30);
        assert_eq!(openai.total_tokens, 150);

        // Ollama 原生字段名兼容。
        let ollama = parse_usage_value(&json!({
            "prompt_eval_count": 40,
            "eval_count": 12,
        }));
        assert_eq!(ollama.prompt_tokens, 40);
        assert_eq!(ollama.completion_tokens, 12);
        assert_eq!(ollama.total_tokens, 52);

        assert_eq!(parse_usage_value(&Value::Null), TokenUsage::default());
    }

    #[test]
    fn token_usage_arithmetic_and_cost_estimate() {
        let mut usage = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };
        usage.add(&TokenUsage {
            prompt_tokens: 200,
            completion_tokens: 30,
            total_tokens: 230,
        });
        assert_eq!(usage.total_tokens, 380);

        let before = TokenUsage {
            prompt_tokens: 300,
            completion_tokens: 80,
            total_tokens: 380,
        };
        let delta = usage.saturating_sub(&before);
        assert_eq!(delta.total_tokens, 0);

        let delta = before.saturating_sub(&TokenUsage::default());
        assert_eq!(delta.prompt_tokens, 300);
        assert!((delta.cost_estimate_usd(2.0, 8.0) - 0.00124).abs() < 1e-9);
    }

    #[test]
    fn budget_violation_blocks_when_caps_exceeded() {
        let usage = TokenUsage {
            prompt_tokens: 900,
            completion_tokens: 200,
            total_tokens: 1100,
        };
        assert!(budget_violation(&usage, None, None, 0.0, 0.0).is_none());
        assert!(budget_violation(&usage, Some(2000), None, 0.0, 0.0).is_none());
        let violation =
            budget_violation(&usage, Some(1000), None, 0.0, 0.0).expect("token 超限应熔断");
        assert!(violation.contains("用量预算"));
        assert!(violation.contains("1100"));

        let cost = budget_violation(&usage, None, Some(0.001), 2.0, 8.0).expect("成本超限应熔断");
        assert!(cost.contains("成本预算"));

        // 未到成本上限不熔断：0.0006+0.0016=0.0022 < 0.01。
        assert!(budget_violation(&usage, None, Some(0.01), 0.5, 2.0).is_none());
    }

    #[test]
    fn parse_sse_payload_extracts_trailing_usage_block() {
        let payload = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let delta = parse_sse_payload(payload).expect("usage 块应返回 Some");
        assert_eq!(delta.content, None);
        let usage = delta.usage.expect("usage 应被解析");
        assert_eq!(usage.total_tokens, 15);

        // 无 usage 的空 delta 仍按心跳忽略。
        assert!(
            parse_sse_payload(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#).is_none()
        );
    }

    #[tokio::test]
    async fn cloud_disabled_rejects_requests_before_network() {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENAI_API_KEY", "test");
        std::env::set_var("OPENAI_BASE_URL", "https://api.example.com/v1");
        std::env::set_var("OPENAI_MODEL", "mock");
        std::env::set_var("OWO_CLOUD_ENABLED", "false");
        let config = OpenAiCompatibleConfig::from_env().unwrap();
        assert!(!config.cloud_enabled);
        let provider = OpenAiCompatibleProvider::new(config).unwrap();
        let error = provider.complete(&[], &[]).await.unwrap_err();
        assert!(error.contains("数据出境"));
        std::env::remove_var("OWO_CLOUD_ENABLED");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENAI_BASE_URL");
        std::env::remove_var("OPENAI_MODEL");
    }

    #[tokio::test]
    async fn cloud_switch_applies_without_reconstruction() {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENAI_API_KEY", "test");
        std::env::set_var("OPENAI_BASE_URL", "https://api.example.com/v1");
        std::env::set_var("OPENAI_MODEL", "mock");
        std::env::remove_var("OWO_CLOUD_ENABLED");
        let config = OpenAiCompatibleConfig::from_env().unwrap();
        assert!(config.cloud_enabled);
        let provider = OpenAiCompatibleProvider::new(config).unwrap();
        assert!(provider.cloud_enabled());
        std::env::set_var("OWO_CLOUD_ENABLED", "false");
        let error = provider.complete(&[], &[]).await.unwrap_err();
        assert!(error.contains("数据出境"));
        std::env::remove_var("OWO_CLOUD_ENABLED");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENAI_BASE_URL");
        std::env::remove_var("OPENAI_MODEL");
    }

    #[tokio::test]
    async fn local_endpoint_does_not_require_key_or_cloud_switch() {
        let _guard = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:11434/v1");
        std::env::set_var("OWO_CLOUD_ENABLED", "false");

        let config = OpenAiCompatibleConfig::from_env().unwrap();
        assert!(config.api_key.is_empty());
        let provider = OpenAiCompatibleProvider::new(config).unwrap();
        assert!(provider.cloud_enabled());

        std::env::remove_var("OWO_CLOUD_ENABLED");
        std::env::remove_var("OPENAI_BASE_URL");
    }

    #[test]
    fn stream_request_includes_usage_option() {
        let provider = OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            api_key: String::new(),
            model: "local".to_string(),
            cloud_enabled: false,
        })
        .unwrap();
        let body = provider.request_body(&[], &[], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn request_body_sends_reasoning_effort_only_for_known_levels() {
        let _guard = crate::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let provider = OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            api_key: String::new(),
            model: "local".to_string(),
            cloud_enabled: false,
        })
        .unwrap();

        // 默认（未选择档位）：请求体与旧版完全一致，不新增字段。
        std::env::remove_var("OWO_REASONING_EFFORT");
        let body = provider.request_body(&[], &[], false);
        assert!(
            body.get("reasoning_effort").is_none(),
            "默认不应下发推理档位"
        );

        std::env::set_var("OWO_REASONING_EFFORT", " HIGH ");
        let body = provider.request_body(&[], &[], false);
        assert_eq!(body["reasoning_effort"], "high");

        // 非法取值不下发：宁可回落模型默认，也不让端点因未知字段报错。
        std::env::set_var("OWO_REASONING_EFFORT", "unsupported");
        let body = provider.request_body(&[], &[], false);
        assert!(body.get("reasoning_effort").is_none(), "非法取值不应下发");
        std::env::remove_var("OWO_REASONING_EFFORT");
    }

    #[test]
    fn utf8_chunks_are_reassembled_without_replacement_characters() {
        let mut buffer = String::new();
        let mut pending = Vec::new();
        let bytes = "中".as_bytes();
        append_utf8_chunk(&mut buffer, &mut pending, &bytes[..1]);
        append_utf8_chunk(&mut buffer, &mut pending, &bytes[1..]);
        assert_eq!(buffer, "中");
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn model_selection_prefers_config_and_hot_switches_via_config() {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENAI_API_KEY", "test");
        std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:9");
        std::env::set_var("OPENAI_MODEL", "model-a");
        std::env::remove_var("OWO_CLOUD_ENABLED");

        // ① 已构造的 provider 用配置里的模型；env 后改不再压过它。
        // 旧行为是 request_body 每次直读 OPENAI_MODEL，导致设置页/--model 选的模型永远被 env 覆盖。
        let config = OpenAiCompatibleConfig::from_env().unwrap();
        let provider = OpenAiCompatibleProvider::new(config).unwrap();
        let body = provider.request_body(&[], &[], false);
        assert_eq!(body["model"], "model-a");
        std::env::set_var("OPENAI_MODEL", "model-b");
        let body = provider.request_body(&[], &[], false);
        assert_eq!(
            body["model"], "model-a",
            "已构造的 provider 不应被 env 改动影响"
        );

        // ② 取值优先级：CLI `--model`（显式覆盖）> 设置页保存值 > OPENAI_MODEL > 内置默认。
        // 热切换靠 DeferredProvider 每次调用重读本函数并重建 provider（无需重启）。
        set_provider_override(None, None, Some("model-settings".to_string()));
        let from_override = OpenAiCompatibleConfig::from_env().unwrap();
        assert_eq!(
            from_override.model, "model-settings",
            "保存值应优先于 OPENAI_MODEL"
        );
        set_model_override("model-cli");
        // 之后再保存设置也不该把它冲掉（CLI 里 build_agent 会再调一次 apply_provider_override）。
        set_provider_override(None, None, Some("model-settings-2".to_string()));
        let from_cli = OpenAiCompatibleConfig::from_env().unwrap();
        assert_eq!(
            from_cli.model, "model-cli",
            "--model 应压过保存值且不被后续保存覆盖"
        );
        clear_model_override();
        set_provider_override(None, None, None);
        let from_env_only = OpenAiCompatibleConfig::from_env().unwrap();
        assert_eq!(
            from_env_only.model, "model-b",
            "没有保存值时回退 OPENAI_MODEL"
        );
        std::env::remove_var("OPENAI_MODEL");
        let from_default = OpenAiCompatibleConfig::from_env().unwrap();
        assert_eq!(from_default.model, DEFAULT_MODEL, "都没有时用内置默认");

        std::env::remove_var("OWO_CLOUD_ENABLED");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENAI_BASE_URL");
        std::env::remove_var("OPENAI_MODEL");
    }

    #[test]
    fn provider_creates_direct_client_when_proxy_configured() {
        let _guard = ENV_LOCK.blocking_lock();
        let proxy_envs = [
            "OWO_HTTP_PROXY",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "https_proxy",
            "http_proxy",
        ];
        let previous: Vec<_> = proxy_envs
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in proxy_envs {
            std::env::remove_var(name);
        }
        std::env::set_var("OWO_HTTP_PROXY", "http://127.0.0.1:9");
        let config = OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:9/v1".to_string(),
            api_key: "test".to_string(),
            model: "test".to_string(),
            cloud_enabled: true,
        };
        let provider = OpenAiCompatibleProvider::new(config).expect("客户端创建成功");
        assert!(provider.direct_client.is_some());
        std::env::remove_var("OWO_HTTP_PROXY");
        let config = OpenAiCompatibleConfig {
            base_url: "http://127.0.0.1:9/v1".to_string(),
            api_key: "test".to_string(),
            model: "test".to_string(),
            cloud_enabled: true,
        };
        let provider = OpenAiCompatibleProvider::new(config).expect("客户端创建成功");
        assert!(provider.direct_client.is_none());
        for (name, value) in previous {
            if let Some(value) = value {
                std::env::set_var(name, value);
            }
        }
    }
}
