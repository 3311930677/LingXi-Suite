//! Pluggable candidate reranking.
//!
//! The engine produces candidates ranked by dictionary frequency and coverage.
//! Reranking is where *context* refines that order — exactly the seam where a
//! neural reranker (e.g. a small language model scoring `preceding_text +
//! candidate`) would later plug in. Keeping it a trait means the engine never
//! depends on any particular ranking model, mirroring the crate's overall
//! "generation is fixed, ranking is pluggable" design.
//!
//! Two lightweight, deterministic rerankers ship here:
//! - [`FrequencyReranker`]: a no-op that trusts the engine's frequency order.
//! - [`PrefixContextReranker`]: a cheap heuristic that boosts candidates whose
//!   text reads naturally after `preceding_text` (avoids immediate repetition,
//!   rewards longer coverage), standing in for a real n-gram/LM reranker.

use crate::{Candidate, InputContext};

/// Reorders (and may rescore) engine candidates using context. Implementations
/// must be pure and order-stable for a given input so results are testable.
pub trait CandidateReranker: Send + Sync {
    /// Short identifier for logs/UI.
    fn name(&self) -> &str;

    /// Rerank `candidates` in place given `context`. The engine has already
    /// sorted them best-first by its own score; a reranker adjusts `score` and
    /// re-sorts. It must not add or drop candidates, only reorder/rescore.
    fn rerank(&self, candidates: &mut Vec<Candidate>, context: &InputContext);
}

/// Trusts the engine's frequency-based order; a well-defined no-op baseline
/// (the "rime-ice native order" against which any neural reranker is A/B'd).
#[derive(Debug, Default, Clone, Copy)]
pub struct FrequencyReranker;

impl CandidateReranker for FrequencyReranker {
    fn name(&self) -> &str {
        "frequency"
    }

    fn rerank(&self, _candidates: &mut Vec<Candidate>, _context: &InputContext) {
        // Intentionally empty: the engine's ordering is the baseline.
    }
}

/// A cheap context reranker. It nudges scores using two signals that a real
/// n-gram or neural model would capture more richly, and is here to prove the
/// seam works end-to-end and to give sensible offline behavior:
///
/// 1. **Anti-repetition**: penalize a candidate that merely repeats the
///    character just committed (`妈` right after `妈` is usually wrong).
/// 2. **Coverage reward**: gently favor candidates that consume more pinyin, so
///    a full-sentence reading outranks a one-character fragment at equal
///    frequency footing.
#[derive(Debug, Clone, Copy)]
pub struct PrefixContextReranker {
    /// Multiplicative penalty (`<1.0`) applied when a candidate starts by
    /// repeating the last committed character.
    pub repeat_penalty: f64,
    /// Additive bonus per extra syllable covered.
    pub coverage_bonus: f64,
}

impl Default for PrefixContextReranker {
    fn default() -> Self {
        Self {
            repeat_penalty: 0.6,
            // Candidate generation already rewards complete words. Keeping the
            // default at zero avoids mistakenly rewarding a bad pinyin split
            // merely because it contains more fragments (`su o yi` > `suo yi`).
            coverage_bonus: 0.0,
        }
    }
}

impl CandidateReranker for PrefixContextReranker {
    fn name(&self) -> &str {
        "prefix-context"
    }

    fn rerank(&self, candidates: &mut Vec<Candidate>, context: &InputContext) {
        let last_committed = context.preceding_text.chars().last();
        for candidate in candidates.iter_mut() {
            let mut score = candidate.score;
            if let Some(last) = last_committed {
                if candidate.text.starts_with(last) {
                    score *= self.repeat_penalty;
                }
            }
            // Coverage reward is relative to the score magnitude so it nudges
            // ties without swamping genuine frequency differences.
            score += candidate.coverage() as f64 * self.coverage_bonus * candidate.score.abs();
            candidate.score = score;
        }
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.coverage().cmp(&a.coverage()))
                .then_with(|| a.text.cmp(&b.text))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(text: &str, syllables: &[&str], score: f64) -> Candidate {
        Candidate {
            text: text.to_string(),
            syllables: syllables.iter().map(|s| s.to_string()).collect(),
            score,
        }
    }

    #[test]
    fn frequency_reranker_is_noop() {
        let mut cs = vec![cand("好", &["hao"], 8000.0), cand("号", &["hao"], 3000.0)];
        let before = cs.clone();
        FrequencyReranker.rerank(&mut cs, &InputContext::default());
        assert_eq!(cs, before);
    }

    #[test]
    fn prefix_context_penalizes_immediate_repetition() {
        let mut cs = vec![cand("妈", &["ma"], 1000.0), cand("吗", &["ma"], 990.0)];
        let ctx = InputContext {
            preceding_text: "妈".to_string(),
            max_candidates: 0,
        };
        PrefixContextReranker::default().rerank(&mut cs, &ctx);
        // `妈` repeats the last char and should be demoted below `吗`.
        assert_eq!(cs[0].text, "吗");
    }

    #[test]
    fn prefix_context_rewards_coverage_on_ties() {
        let mut cs = vec![
            cand("你", &["ni"], 1000.0),
            cand("你好", &["ni", "hao"], 1000.0),
        ];
        PrefixContextReranker {
            coverage_bonus: 0.05,
            ..Default::default()
        }
        .rerank(&mut cs, &InputContext::default());
        assert_eq!(cs[0].text, "你好");
    }
}
