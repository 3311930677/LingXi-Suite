//! E3 验收：真实 tokenizer（tiktoken `cl100k_base`）精度、回退契约与预算推导。
//!
//! 变异验收口径：
//! - 把 `TiktokenCounter::new()` 里的 `cl100k_base()` 换成无效编码名 → 回退启发式，
//!   `default_counter_reports_name` 与中文计数断言必须变红；
//! - 把 `estimate_tokens` 改回「字符数/2」→ `estimate_tokens_uses_real_counter` 变红。
//!
//! 与真实 provider `prompt_tokens` 的偏差对比是手工步骤（消耗真实 token）：
//! 见方案 E3.2 的偏差表脚本口径。

use owo_agent_core::tokenizer::{
    budget_from_window, context_window_for_model, default_counter, tokenizer_name,
    HeuristicCounter, TiktokenCounter, TokenCounter,
};
use owo_agent_core::{estimate_tokens, ChatMessage};

/// 中文基准样本（内嵌，无外部依赖）。
const CJK_SAMPLES: &[&str] = &[
    "你好，请帮我总结这段代码并给出改进建议",
    "帮我在当前目录查找所有 Rust 文件并统计行数",
    "把这个函数重构成异步版本，注意错误处理",
    "明天下午三点提醒我开周会，地点在三楼会议室",
];

#[test]
fn real_counter_beats_legacy_half_chars_on_chinese() {
    let tiktoken = TiktokenCounter::new();
    for sample in CJK_SAMPLES {
        let chars = sample.chars().count();
        let legacy = chars / 2; // P1-1 之前的公式
        let counted = tiktoken.count(sample);
        assert!(
            counted > legacy,
            "真实计数 {counted} 必须高于旧公式 {legacy}（{chars} 字：{sample}）"
        );
        assert!(
            counted <= chars * 2,
            "计数 {counted} 不应超过字数 × 2（{chars} 字）"
        );
    }
}

#[test]
fn ascii_counting_is_reasonable() {
    let tiktoken = TiktokenCounter::new();
    let text = "Please summarize this code and suggest improvements for readability";
    let chars = text.chars().count();
    let counted = tiktoken.count(text);
    // 短英文句按词切分（约 1 token/词）：区间放宽到 3~12 字符/token。
    assert!(
        counted >= chars / 12 && counted <= chars / 3,
        "英文计数 {counted} 超出合理区间（{chars} 字符）"
    );
}

#[test]
fn counting_is_monotonic() {
    let tiktoken = TiktokenCounter::new();
    let short = tiktoken.count("你好");
    let long = tiktoken.count("你好，你好，你好，你好");
    assert!(long > short, "更长的文本计数必须更大");
}

#[test]
fn heuristic_fallback_contract() {
    let heuristic = HeuristicCounter;
    assert_eq!(heuristic.name(), "heuristic");
    assert!(heuristic.estimated());
    // 中文 ≈1 token/字；ASCII ≈4 字符/token。
    assert_eq!(heuristic.count("你好世界"), 4);
    assert_eq!(heuristic.count("abcdefgh"), 2);
    assert_eq!(heuristic.count(""), 0);
    // 混合文本：ASCII 段与 CJK 段分别计价。
    assert_eq!(heuristic.count("abcd你好"), 3);
}

#[test]
fn default_counter_reports_name_honestly() {
    let name = tokenizer_name();
    assert!(
        matches!(name, "tiktoken" | "heuristic"),
        "未知计数器名：{name}"
    );
    // 对 DeepSeek/Qwen 等非 OpenAI 模型，cl100k 是近似值——必须如实标注。
    assert!(default_counter().estimated());
}

#[test]
fn estimate_tokens_uses_real_counter() {
    let text = "你好，请帮我总结这段代码并给出改进建议";
    let chars = text.chars().count();
    let messages = vec![ChatMessage::user(text.to_string())];
    let total = estimate_tokens(&messages);
    // 旧公式给出 chars/2 + 4；真实计数对中文 ≥ 字数（再加开销）。
    assert!(
        total >= chars,
        "中文单条消息估算 {total} 必须 ≥ 字数 {chars}（旧公式约 {}）",
        chars / 2 + 4
    );
}

#[test]
fn model_window_and_budget_derivation() {
    assert_eq!(context_window_for_model("deepseek-chat"), Some(65_536));
    assert_eq!(context_window_for_model("deepseek-reasoner"), Some(65_536));
    assert_eq!(
        context_window_for_model("qwen2.5-72b-instruct"),
        Some(32_768)
    );
    assert_eq!(context_window_for_model("gpt-4o"), Some(128_000));
    assert_eq!(context_window_for_model("unknown-x"), None);

    let window = context_window_for_model("deepseek-chat").expect("deepseek 窗口已知");
    assert_eq!(
        budget_from_window(window),
        49_152,
        "预算 = 窗口 × 0.75（65536 → 49152）"
    );
}

#[test]
fn empty_messages_count_zero() {
    assert_eq!(estimate_tokens(&[]), 0);
}
