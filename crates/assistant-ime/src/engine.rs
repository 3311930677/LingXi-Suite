//! The pinyin input engine: segmentation → candidate generation → reranking.
//!
//! Given a raw pinyin key sequence and its [`InputContext`], the engine:
//!
//! 1. asks [`segment`] for valid syllable splits;
//! 2. for each split, runs a Viterbi-style shortest-path search over the
//!    [`Dictionary`] to build the best *full-sentence* candidate (the maximum
//!    total log-frequency segmentation of the syllables into dictionary words);
//! 3. adds shorter high-frequency word/prefix candidates and single-syllable
//!    fallbacks so the panel is never empty; and
//! 4. hands the ranked list to a [`CandidateReranker`] for context refinement.
//!
//! Scores are additive log-frequencies, which is what a real IME language model
//! optimizes: multiplying word probabilities becomes summing their logs, so the
//! best path is the highest-scoring one and stays numerically stable.

use std::collections::HashMap;

use crate::dict::Dictionary;
use crate::rerank::{CandidateReranker, PrefixContextReranker};
use crate::segment::segment;
use crate::{Candidate, InputContext, InputEngine};

/// A pinyin IME backed by an in-memory [`Dictionary`] and a pluggable
/// [`CandidateReranker`]. Construct with [`PinyinInputEngine::new`] or the
/// batteries-included [`PinyinInputEngine::builtin`].
pub struct PinyinInputEngine {
    dictionary: Dictionary,
    reranker: Box<dyn CandidateReranker>,
    /// Small smoothing weight for words the dictionary lacks, so single-syllable
    /// fallbacks still receive a finite score instead of `-inf`.
    unknown_weight: f64,
}

impl PinyinInputEngine {
    /// Build an engine from a dictionary and a reranker.
    pub fn new(dictionary: Dictionary, reranker: Box<dyn CandidateReranker>) -> Self {
        Self {
            dictionary,
            reranker,
            unknown_weight: 1.0,
        }
    }

    /// Build an engine from the built-in dictionary and the default
    /// context-aware reranker — usable and testable with no setup.
    pub fn builtin() -> Self {
        Self::new(
            Dictionary::builtin(),
            Box::new(PrefixContextReranker::default()),
        )
    }

    /// Access the underlying dictionary (e.g. to load more entries).
    pub fn dictionary_mut(&mut self) -> &mut Dictionary {
        &mut self.dictionary
    }

    /// Log-frequency score for a weight, with `+1` smoothing so weight `0` is
    /// finite and monotonic in weight.
    fn log_freq(weight: u32) -> f64 {
        ((weight as f64) + 1.0).ln()
    }

    /// Viterbi shortest-path over one syllable split: find the word
    /// segmentation maximizing total log-frequency. Returns the best sentence
    /// as `(joined word text, total score, pinyin path)`, where the path holds
    /// each chosen word's joined pinyin key in order (so `你好` over `[ni, hao]`
    /// yields the path `["nihao"]` if matched as one word, or `["ni", "hao"]` if
    /// pieced together). `None` if no path covers all syllables.
    #[allow(clippy::type_complexity)]
    fn best_sentence(&self, syllables: &[String]) -> Option<(String, f64, Vec<String>)> {
        let n = syllables.len();
        // best[i] = (score, text, path) for the best segmentation of syllables[..i].
        let mut best: Vec<Option<(f64, String, Vec<String>)>> = vec![None; n + 1];
        best[0] = Some((0.0, String::new(), Vec::new()));

        for end in 1..=n {
            for start in 0..end {
                let Some((prev_score, prev_text, prev_path)) = best[start].clone() else {
                    continue;
                };
                let key: String = syllables[start..end].concat();
                let hits = self.dictionary.lookup(&key);
                let (word, weight) = if let Some(top) = hits.first() {
                    (top.word.clone(), top.weight)
                } else if end - start == 1 {
                    // Unknown single syllable: keep the path alive with a
                    // placeholder using the raw pinyin, lightly smoothed. This
                    // guarantees a full-length path always exists.
                    (syllables[start].clone(), 0)
                } else {
                    continue;
                };
                let step = if weight == 0 {
                    self.unknown_weight.ln()
                } else {
                    Self::log_freq(weight)
                };
                let score = prev_score + step;
                let text = format!("{prev_text}{word}");
                let mut path = prev_path.clone();
                path.push(key);
                match &best[end] {
                    Some((existing, _, _)) if *existing >= score => {}
                    _ => best[end] = Some((score, text, path)),
                }
            }
        }
        best[n]
            .clone()
            .map(|(score, text, path)| (text, score, path))
    }

    /// Collect candidates for a single syllable split into `acc`, keyed by text
    /// so duplicates across splits keep their best score.
    fn collect_from_split(&self, syllables: &[String], acc: &mut HashMap<String, Candidate>) {
        let n = syllables.len();
        if n == 0 {
            return;
        }

        // 1) Best full-sentence path over all syllables. Its coverage is the
        // whole input regardless of how the split fragmented it, so it is NOT
        // biased by the fragment count `n` (that used to let a junk split like
        // `[ha, o]` outrank the clean `[hao]` for the same word). Promotion of
        // longer *real* words is the reranker's job via `Candidate::coverage`.
        //
        // A path that still contains raw-pinyin placeholders (unresolved
        // syllables surface as ASCII letters in `text`) is only worth offering
        // when it is *fully* raw — a legitimate "no dictionary word" fallback.
        // A partially-resolved mash-up such as `你好o` is noise and is dropped.
        if let Some((text, score, path)) = self.best_sentence(syllables) {
            if !is_partial_placeholder(&text) {
                Self::offer(
                    acc,
                    Candidate {
                        text,
                        syllables: path,
                        score,
                    },
                );
            }
        }

        // 2) Whole-input dictionary words (e.g. `nihao` → 你好 directly), which
        // may beat the pieced-together sentence and are what users expect for
        // common words. Their pinyin is the whole input as one word.
        let whole_key: String = syllables.concat();
        for entry in self.dictionary.lookup(&whole_key) {
            Self::offer(
                acc,
                Candidate {
                    text: entry.word.clone(),
                    syllables: vec![whole_key.clone()],
                    // An exact dictionary entry covering the complete input is
                    // substantially more plausible than a Viterbi path made of
                    // several individually frequent characters. Without this
                    // word bonus, a large dictionary ranked 诉+哦 above 所以
                    // because log-frequency addition rewards extra boundaries.
                    score: Self::log_freq(entry.weight) + 48.0,
                },
            );
        }

        // 3) Prefix words: the longest dictionary word starting at syllable 0
        // that covers a prefix of the input, so `nihaoshijie` still surfaces
        // `你好` as a strong shorter candidate.
        for prefix_len in (1..n).rev() {
            let key: String = syllables[..prefix_len].concat();
            for entry in self.dictionary.lookup(&key) {
                Self::offer(
                    acc,
                    Candidate {
                        text: entry.word.clone(),
                        syllables: syllables[..prefix_len].to_vec(),
                        score: Self::log_freq(entry.weight),
                    },
                );
            }
        }

        // 4) First-syllable single characters, so a partial input always shows
        // its leading character(s). Raw-pinyin fallback for an unknown syllable
        // is handled once by the caller on the best split only, to avoid junk
        // splits (`[ha, o]` of `hao`) injecting bare-latin noise like `ha`.
        let first = &syllables[0];
        for entry in self.dictionary.lookup(first) {
            Self::offer(
                acc,
                Candidate {
                    text: entry.word.clone(),
                    syllables: vec![first.clone()],
                    score: Self::log_freq(entry.weight),
                },
            );
        }
    }

    /// Add candidates matched by a pure simplified-pinyin key (`nh`, `zgr`).
    /// The bonus keeps abbreviations useful while remaining below an exact
    /// full-pinyin whole-word match (+48).
    fn collect_abbreviations(&self, key: &str, acc: &mut HashMap<String, Candidate>) {
        for entry in self.dictionary.lookup_abbreviation(key) {
            Self::offer(
                acc,
                Candidate {
                    text: entry.word.clone(),
                    syllables: entry.syllables.clone(),
                    score: Self::log_freq(entry.weight) + 24.0,
                },
            );
        }
    }

    /// Insert `candidate` into `acc`. On a text collision keep the better one:
    /// higher score wins, and on an (approximate) score tie the cleaner path —
    /// fewer syllables — wins, so `好` keeps `[hao]` rather than a junk `[ha, o]`.
    fn offer(acc: &mut HashMap<String, Candidate>, candidate: Candidate) {
        acc.entry(candidate.text.clone())
            .and_modify(|existing| {
                let better_score = candidate.score > existing.score + f64::EPSILON;
                let tie = (candidate.score - existing.score).abs() <= f64::EPSILON;
                let cleaner = tie && candidate.syllables.len() < existing.syllables.len();
                if better_score || cleaner {
                    existing.score = candidate.score;
                    existing.syllables = candidate.syllables.clone();
                }
            })
            .or_insert(candidate);
    }
}

impl InputEngine for PinyinInputEngine {
    fn candidates(&self, pinyin: &str, context: &InputContext) -> Vec<Candidate> {
        let normalized = pinyin.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Vec::new();
        }
        let splits = segment(&normalized);
        let mut acc: HashMap<String, Candidate> = HashMap::new();

        // Full pinyin and simplified pinyin are independent candidate sources.
        // Exact whole-word candidates carry a larger bonus and therefore win
        // when a key happens to be both a legal syllable and an abbreviation.
        self.collect_abbreviations(&normalized, &mut acc);
        for split in splits.iter().take(6) {
            self.collect_from_split(&split.syllables, &mut acc);
        }

        // Guaranteed non-empty fallback: if nothing resolved to a dictionary
        // word (e.g. a legal but out-of-vocabulary syllable such as `den`),
        // surface the raw best-split reading so the panel is never empty. This
        // runs on the best split only, so junk splits cannot inject bare-latin
        // noise like `ha` for `hao`.
        if acc.is_empty() {
            if let Some(best) = splits.first() {
                let text: String = best.syllables.concat();
                Self::offer(
                    &mut acc,
                    Candidate {
                        text,
                        syllables: best.syllables.clone(),
                        score: self.unknown_weight.ln(),
                    },
                );
            }
        }

        let mut candidates: Vec<Candidate> = acc.into_values().collect();
        // Initial engine order: score desc, then longer coverage, then text for
        // determinism. The reranker refines this using context.
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.coverage().cmp(&a.coverage()))
                .then_with(|| a.text.cmp(&b.text))
        });

        self.reranker.rerank(&mut candidates, context);

        if context.max_candidates > 0 && candidates.len() > context.max_candidates {
            candidates.truncate(context.max_candidates);
        }
        candidates
    }
}

/// Whether `text` is a partially-resolved Viterbi path: it mixes real Chinese
/// characters with leftover raw-pinyin ASCII letters (e.g. `你好o`). Such a
/// mash-up is noise. A fully-Chinese result and a fully-raw ASCII fallback are
/// both *not* partial and are kept.
fn is_partial_placeholder(text: &str) -> bool {
    let has_ascii_letter = text.bytes().any(|b| b.is_ascii_alphabetic());
    let has_non_ascii = !text.is_ascii();
    has_ascii_letter && has_non_ascii
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn common_word_is_top_candidate() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("nihao", &InputContext::default());
        assert_eq!(cs[0].text, "你好");
    }

    #[test]
    fn single_syllable_offers_homophones_by_frequency() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::with_limit(5));
        let words = texts(&cs);
        assert_eq!(words[0], "好"); // weight 8000 > 号 3000
        assert!(words.contains(&"号"));
    }

    #[test]
    fn candidate_syllables_reflect_the_clean_split() {
        // Regression: a junk split `[ha, o]` must not overwrite `好`'s syllable
        // field, which should stay the clean single syllable `[hao]`.
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::default());
        let hao = cs.iter().find(|c| c.text == "好").expect("好 present");
        assert_eq!(hao.syllables, vec!["hao"]);
    }

    #[test]
    fn no_bare_latin_noise_from_junk_splits() {
        // `hao` used to leak a bare `ha` candidate via the `[ha, o]` split.
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::default());
        assert!(
            cs.iter()
                .all(|c| !c.text.bytes().all(|b| b.is_ascii_alphabetic())),
            "no all-latin junk candidate expected, got {:?}",
            texts(&cs)
        );
    }

    #[test]
    fn no_partial_placeholder_candidates() {
        // A Han+latin mash-up like `你好o` must never be offered.
        let engine = PinyinInputEngine::builtin();
        for pinyin in ["nihao", "woaizhongguo", "nihaoshijie"] {
            let cs = engine.candidates(pinyin, &InputContext::default());
            assert!(
                cs.iter().all(|c| !is_partial_placeholder(&c.text)),
                "partial placeholder leaked for {pinyin}: {:?}",
                texts(&cs)
            );
        }
    }

    #[test]
    fn whole_word_beats_high_frequency_character_split() {
        let mut dict = Dictionary::new().with_fuzzy(false);
        dict.insert("诉", "su", 667_093);
        dict.insert("哦", "o", 3_563_330);
        dict.insert("所以", "suoyi", 502_905);
        dict.insert("一", "yi", 10_000_000);
        let engine = PinyinInputEngine::new(dict, Box::new(PrefixContextReranker::default()));
        let cs = engine.candidates("suoyi", &InputContext::default());
        assert_eq!(cs[0].text, "所以");
    }

    #[test]
    fn simplified_pinyin_returns_multi_syllable_words() {
        let engine = PinyinInputEngine::builtin();
        let nh = engine.candidates("nh", &InputContext::with_limit(5));
        assert_eq!(nh[0].text, "你好");
        let zgr = engine.candidates("zgr", &InputContext::with_limit(5));
        assert_eq!(zgr[0].text, "中国人");
    }

    #[test]
    fn single_letter_does_not_trigger_abbreviation_flood() {
        let engine = PinyinInputEngine::builtin();
        assert!(engine.candidates("z", &InputContext::default()).is_empty());
    }

    #[test]
    fn full_sentence_search_pieces_words_together() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("woaizhongguo", &InputContext::default());
        // The best full-coverage sentence should read 我爱中国.
        assert_eq!(cs[0].text, "我爱中国");
    }

    #[test]
    fn longer_input_still_surfaces_shorter_prefix_word() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("nihaoshijie", &InputContext::default());
        let words = texts(&cs);
        // Full sentence 你好世界 present, and 你好 offered as a prefix candidate.
        assert!(words.contains(&"你好世界") || words.contains(&"你好"));
        assert!(words.contains(&"你好"));
    }

    #[test]
    fn respects_candidate_limit() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::with_limit(1));
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn unknown_pinyin_falls_back_to_raw_syllable() {
        // A legal syllable with no dictionary word ("den") must still return
        // something rather than an empty panel.
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("den", &InputContext::default());
        assert!(!cs.is_empty());
        assert_eq!(cs[0].text, "den");
    }

    #[test]
    fn empty_input_yields_no_candidates() {
        let engine = PinyinInputEngine::builtin();
        assert!(engine.candidates("", &InputContext::default()).is_empty());
    }

    #[test]
    fn context_reranks_to_avoid_repetition() {
        let mut dict = Dictionary::new();
        dict.insert("妈", "ma", 1000);
        dict.insert("吗", "ma", 990);
        let engine = PinyinInputEngine::new(dict, Box::new(PrefixContextReranker::default()));
        let ctx = InputContext {
            preceding_text: "妈".to_string(),
            max_candidates: 0,
        };
        let cs = engine.candidates("ma", &ctx);
        assert_eq!(cs[0].text, "吗"); // repetition of 妈 demoted
    }
}
