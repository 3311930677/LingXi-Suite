//! Pure-Rust pinyin input-method engine core.
//!
//! This crate implements the heart of a real pinyin IME — the part that turns a
//! raw pinyin key sequence such as `nihaoshijie` into ranked Chinese candidates
//! (`你好世界`, `你好`, `你`, …). It is intentionally free of any C dependency
//! (no librime, no ICU) so the whole thing builds and is unit-tested under the
//! GNU toolchain like the rest of the workspace, with no real desktop needed.
//!
//! # Layering
//!
//! The pipeline mirrors how librime is structured, so a production build can
//! later swap the internals for librime + rime-ice behind the same
//! [`InputEngine`] trait without touching callers:
//!
//! ```text
//! raw pinyin ─▶ segmentation ─▶ candidate generation ─▶ reranking ─▶ candidates
//!   "nihao"      [ni][hao]        今/你 + 好 …            context bias
//! ```
//!
//! - [`segment`] splits a key sequence into pinyin syllables (with fuzzy/typo
//!   tolerance).
//! - [`dict`] holds the word ↔ pinyin ↔ frequency data and answers prefix
//!   queries.
//! - [`engine`] runs a shortest-path (Viterbi-style) sentence search over the
//!   dictionary and produces ranked [`Candidate`]s.
//! - [`rerank`] is a pluggable post-processor: a small n-gram/context reranker
//!   ships here, and a neural reranker can implement the same trait later.
//!
//! Nothing here performs I/O by default: a compact built-in dictionary makes the
//! engine usable and testable offline, while [`dict::Dictionary::load_text`]
//! ingests a rime-ice-style `word<TAB>pinyin<TAB>weight` table for real use.

pub mod dict;
pub mod engine;
pub mod rerank;
pub mod segment;

pub use dict::{Dictionary, DictionaryEntry};
pub use engine::PinyinInputEngine;
pub use rerank::{CandidateReranker, FrequencyReranker, PrefixContextReranker};
pub use segment::{segment, SyllableSplit};

/// One Chinese candidate proposed for a pinyin input.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The Chinese text of this candidate, e.g. `你好`.
    pub text: String,
    /// The pinyin syllables this candidate consumed, e.g. `["ni", "hao"]`.
    /// Its length tells the caller how much of the input this candidate covers.
    pub syllables: Vec<String>,
    /// Ranking score; higher is better. Combines dictionary frequency and any
    /// reranker adjustments. Only the relative order across candidates for the
    /// same input is meaningful.
    pub score: f64,
}

impl Candidate {
    /// How many pinyin syllables this candidate covers. A full-sentence match
    /// covers every input syllable; a shorter prefix match covers fewer.
    pub fn coverage(&self) -> usize {
        self.syllables.len()
    }
}

/// Immutable context handed to the engine and rerankers: what the user has
/// already committed just before the current pinyin. It lets a reranker prefer
/// candidates that read naturally after the preceding text without the engine
/// depending on any particular ranking model.
#[derive(Debug, Clone, Default)]
pub struct InputContext {
    /// Text already committed immediately to the left of the caret. Empty at the
    /// start of a field.
    pub preceding_text: String,
    /// Maximum number of candidates the caller wants back. `0` means "no limit".
    pub max_candidates: usize,
}

impl InputContext {
    /// Context with only a candidate cap and no preceding text.
    pub fn with_limit(max_candidates: usize) -> Self {
        Self {
            preceding_text: String::new(),
            max_candidates,
        }
    }
}

/// The platform-agnostic input-method engine interface.
///
/// A production build can implement this over librime; the in-crate
/// [`PinyinInputEngine`] implements it in pure Rust. Callers (the desktop
/// candidate panel, a future TSF shell, tests) depend only on this trait.
pub trait InputEngine {
    /// Produce ranked candidates for a raw pinyin key sequence such as
    /// `"nihao"`, given the surrounding [`InputContext`].
    ///
    /// The result is ordered best-first. An empty input yields no candidates.
    fn candidates(&self, pinyin: &str, context: &InputContext) -> Vec<Candidate>;
}
