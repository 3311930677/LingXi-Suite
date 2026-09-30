//! Pluggable text transformation.
//!
//! The product's real value comes from an on-device model, but the platform
//! pipeline must not depend on any particular model runtime. Everything here
//! is expressed against the [`Transformer`] trait: the built-in deterministic
//! transformers below make the end-to-end flow testable today, and an on-device
//! model integration (`assistant-inference`) will implement the same trait
//! later without touching capture/write-back code.

/// A text transformation applied to the selected text.
pub trait Transformer {
    /// Short identifier for logs and UI.
    fn name(&self) -> &str;

    /// Transform the input selection into replacement text.
    fn transform(&self, input: &str) -> String;
}

/// Prepends a fixed marker; a stand-in that visibly proves the pipeline works.
pub struct PrefixTransformer {
    prefix: String,
}

impl PrefixTransformer {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl Transformer for PrefixTransformer {
    fn name(&self) -> &str {
        "prefix"
    }

    fn transform(&self, input: &str) -> String {
        format!("{}{input}", self.prefix)
    }
}

/// Collapses runs of intra-line spaces/tabs and trims each line, preserving
/// line breaks. A realistic, deterministic "tidy whitespace" transformation.
pub struct WhitespaceTidy;

impl Transformer for WhitespaceTidy {
    fn name(&self) -> &str {
        "tidy"
    }

    fn transform(&self, input: &str) -> String {
        input
            .split('\n')
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Uppercases the selection (locale-independent Unicode uppercase).
pub struct UpperCase;

impl Transformer for UpperCase {
    fn name(&self) -> &str {
        "upper"
    }

    fn transform(&self, input: &str) -> String {
        input.to_uppercase()
    }
}

/// Resolve a transformer by mode name for CLI/demo selection.
pub fn transformer_by_name(name: &str) -> Option<Box<dyn Transformer>> {
    match name {
        "prefix" => Some(Box::new(PrefixTransformer::new("[AI] "))),
        "tidy" => Some(Box::new(WhitespaceTidy)),
        "upper" => Some(Box::new(UpperCase)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_prepends_marker() {
        let t = PrefixTransformer::new("[AI] ");
        assert_eq!(t.transform("你好"), "[AI] 你好");
    }

    #[test]
    fn tidy_collapses_spaces_and_preserves_newlines() {
        let t = WhitespaceTidy;
        assert_eq!(t.transform("  a   b \n  c  d "), "a b\nc d");
    }

    #[test]
    fn upper_is_unicode_aware() {
        assert_eq!(UpperCase.transform("aBç"), "ABÇ");
    }

    #[test]
    fn resolve_known_modes_only() {
        assert!(transformer_by_name("prefix").is_some());
        assert!(transformer_by_name("tidy").is_some());
        assert!(transformer_by_name("upper").is_some());
        assert!(transformer_by_name("nope").is_none());
    }
}
