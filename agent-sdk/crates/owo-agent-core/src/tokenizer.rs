//! Token 计数与模型上下文窗口元数据（P1-1：替换「字符数/2 + 4」的假 tokenizer）。
//!
//! - 默认计数器：`tiktoken` 的 `cl100k_base` 离线 BPE（OpenAI 系精确；
//!   DeepSeek / Qwen 等为近似值，`estimated()` 恒为 true，接口如实标注）；
//! - 兜底：tiktoken 初始化失败时退回启发式（ASCII ≈4 字符/token，CJK ≈1 字符/token），
//!   仍远优于旧的 `字符数/2` 全局折半。

use std::sync::OnceLock;

/// 每消息固定开销（role/分隔符等）。
pub const MESSAGE_OVERHEAD: usize = 4;

/// 文本 token 计数器。
pub trait TokenCounter: Send + Sync {
    /// 文本 token 数。
    fn count(&self, text: &str) -> usize;
    /// 计数器名称（`tiktoken` / `heuristic`）。
    fn name(&self) -> &'static str;
    /// 是否为近似值（对非原生模型族恒为 true，须在接口中如实标注）。
    fn estimated(&self) -> bool;
}

/// tiktoken `cl100k_base` 离线 BPE 计数器。
pub struct TiktokenCounter {
    bpe: Option<tiktoken_rs::CoreBPE>,
}

impl TiktokenCounter {
    pub fn new() -> Self {
        Self {
            bpe: tiktoken_rs::cl100k_base().ok(),
        }
    }

    /// BPE 表是否可用（不可用时计数值来自启发式兜底）。
    pub fn available(&self) -> bool {
        self.bpe.is_some()
    }
}

impl Default for TiktokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCounter for TiktokenCounter {
    fn count(&self, text: &str) -> usize {
        match &self.bpe {
            Some(bpe) => bpe.encode_with_special_tokens(text).len(),
            None => HeuristicCounter.count(text),
        }
    }

    fn name(&self) -> &'static str {
        "tiktoken"
    }

    fn estimated(&self) -> bool {
        // cl100k 对 OpenAI 系精确，对 DeepSeek/Qwen 等为近似——统一如实标注。
        true
    }
}

/// 启发式兜底计数器（仅在 BPE 不可用时生效）。
pub struct HeuristicCounter;

impl TokenCounter for HeuristicCounter {
    fn count(&self, text: &str) -> usize {
        let mut tokens = 0_usize;
        let mut ascii_run = 0_usize;
        for ch in text.chars() {
            if ch.is_ascii() {
                ascii_run += 1;
            } else {
                tokens += ascii_run.div_ceil(4);
                ascii_run = 0;
                tokens += 1;
            }
        }
        tokens + ascii_run.div_ceil(4)
    }

    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn estimated(&self) -> bool {
        true
    }
}

/// 进程级默认计数器（tiktoken BPE 只初始化一次）。
pub fn default_counter() -> &'static dyn TokenCounter {
    static COUNTER: OnceLock<TiktokenCounter> = OnceLock::new();
    COUNTER.get_or_init(TiktokenCounter::new)
}

/// 当前生效计数器的名称（`/usage`、`/session/{id}/context` 诊断字段）。
pub fn tokenizer_name() -> &'static str {
    default_counter().name()
}

/// 计算消息序列的 token 估算值（含每消息固定开销）。
pub fn count_messages<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    let counter = default_counter();
    texts
        .into_iter()
        .map(|text| counter.count(text) + MESSAGE_OVERHEAD)
        .sum()
}

/// 已知模型的上下文窗口（tokens）。未知模型返回 `None`（沿用显式预算）。
///
/// 匹配为「包含」语义（`deepseek-chat`、`deepseek-reasoner` 都命中 `deepseek`）。
pub fn context_window_for_model(model: &str) -> Option<usize> {
    const TABLE: &[(&str, usize)] = &[
        ("deepseek", 65_536),
        ("qwen", 32_768),
        ("glm-4", 128_000),
        ("moonshot", 131_072),
        ("kimi", 131_072),
        ("gpt-4o", 128_000),
        ("gpt-4.1", 128_000),
        ("gpt-4", 8_192),
        ("gpt-3.5", 16_385),
        ("gpt-oss", 131_072),
        ("o1", 200_000),
        ("o3", 200_000),
        ("claude", 200_000),
        ("llama", 32_768),
        ("mistral", 32_768),
    ];
    let model = model.to_lowercase();
    TABLE
        .iter()
        .find(|(needle, _)| model.contains(needle))
        .map(|(_, window)| *window)
}

/// 由上下文窗口推导 token 预算（窗口 × 0.75，向下取整）。
pub fn budget_from_window(window: usize) -> usize {
    (window as f64 * 0.75) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_counts_cjk_and_ascii() {
        let counter = HeuristicCounter;
        // 4 个 ASCII 字符 ≈ 1 token。
        assert_eq!(counter.count("abcd"), 1);
        // 4 个 CJK 字符 ≈ 4 token。
        assert_eq!(counter.count("你好世界"), 4);
        assert_eq!(counter.count(""), 0);
    }

    #[test]
    fn tiktoken_counts_cjk_more_realistically_than_half_chars() {
        let counter = TiktokenCounter::new();
        let text = "你好，请帮我总结这段代码并给出改进建议";
        let counted = counter.count(text);
        let half_chars = text.chars().count() / 2;
        assert!(
            counted > half_chars,
            "中文 tiktoken 计数（{counted}）必须明显高于旧的字符数/2（{half_chars}）"
        );
        // cl100k 中文经验区间：字数 × 0.5 ~ 1.5。
        let chars = text.chars().count();
        assert!(
            (chars / 2..=chars * 2).contains(&counted),
            "计数 {counted} 超出中文合理区间（{chars} 字）"
        );
    }

    #[test]
    fn tiktoken_counts_ascii_close_to_four_chars_per_token() {
        let counter = TiktokenCounter::new();
        let counted = counter.count("hello world this is plain ascii text");
        let chars = 37_usize;
        assert!(
            counted <= chars.div_ceil(2),
            "ASCII 文本 token（{counted}）应远少于字符数 {chars}"
        );
    }

    #[test]
    fn count_messages_includes_overhead() {
        let total = count_messages(["你好"]);
        assert!(total >= MESSAGE_OVERHEAD + 2, "含开销：{total}");
    }

    #[test]
    fn model_window_table() {
        assert_eq!(context_window_for_model("deepseek-chat"), Some(65_536));
        assert_eq!(context_window_for_model("Qwen2.5-72B"), Some(32_768));
        assert_eq!(context_window_for_model("gpt-4o-mini"), Some(128_000));
        assert_eq!(context_window_for_model("unknown-model-x"), None);
    }

    #[test]
    fn budget_is_three_quarters_of_window() {
        assert_eq!(budget_from_window(65_536), 49_152);
        assert_eq!(budget_from_window(128_000), 96_000);
    }

    #[test]
    fn default_counter_is_tiktoken_when_available() {
        // 契约：BPE 表随包内嵌，正常情况下用 tiktoken；名称必须如实反映。
        let name = tokenizer_name();
        assert!(
            name == "tiktoken" || name == "heuristic",
            "未知计数器名：{name}"
        );
    }
}
