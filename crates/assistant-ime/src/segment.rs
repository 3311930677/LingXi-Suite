//! Pinyin syllable segmentation.
//!
//! Turning a flat key sequence such as `xian` into syllables is genuinely
//! ambiguous: it can be `xian` (先) or `xi'an` (西安). A real IME therefore does
//! not pick one split — it enumerates the *valid* ones and lets the
//! dictionary/language model decide. This module owns:
//!
//! 1. the set of legal toneless Mandarin pinyin syllables, and
//! 2. a segmenter that returns every full segmentation of the input into legal
//!    syllables, plus (when the tail is still an incomplete syllable) the split
//!    covering the longest legal prefix so a mid-typing user still gets results.
//!
//! Fuzzy-pinyin normalization (`zh↔z`, `in↔ing`, …) is handled at dictionary
//! lookup time rather than here, so segmentation stays purely about *legality*
//! and the fuzzy policy lives in exactly one place.

use std::collections::HashSet;
use std::sync::OnceLock;

/// The legal base pinyin syllables (toneless), space-separated to keep the
/// source compact. This is the standard ~410-syllable Mandarin inventory and is
/// the single source of truth for what counts as a legal syllable. `ü` is
/// written `v` (`lv`, `nv`); `u`-vs-`v` is reconciled at lookup.
const SYLLABLE_TABLE: &str = "\
a ai an ang ao \
ba bai ban bang bao bei ben beng bi bian biao bie bin bing bo bu \
ca cai can cang cao ce cen ceng cha chai chan chang chao che chen cheng chi chong chou chu chua chuai chuan chuang chui chun chuo ci cong cou cu cuan cui cun cuo \
da dai dan dang dao de dei den deng di dia dian diao die ding diu dong dou du duan dui dun duo \
e ei en eng er \
fa fan fang fei fen feng fo fou fu \
ga gai gan gang gao ge gei gen geng gong gou gu gua guai guan guang gui gun guo \
ha hai han hang hao he hei hen heng hong hou hu hua huai huan huang hui hun huo \
ji jia jian jiang jiao jie jin jing jiong jiu ju juan jue jun \
ka kai kan kang kao ke kei ken keng kong kou ku kua kuai kuan kuang kui kun kuo \
la lai lan lang lao le lei leng li lia lian liang liao lie lin ling liu long lou lu luan lue lun luo lv \
ma mai man mang mao me mei men meng mi mian miao mie min ming miu mo mou mu \
na nai nan nang nao ne nei nen neng ni nian niang niao nie nin ning niu nong nou nu nuan nue nuo nv \
o ou \
pa pai pan pang pao pei pen peng pi pian piao pie pin ping po pou pu \
qi qia qian qiang qiao qie qin qing qiong qiu qu quan que qun \
ran rang rao re ren reng ri rong rou ru rua ruan rui run ruo \
sa sai san sang sao se sen seng sha shai shan shang shao she shei shen sheng shi shou shu shua shuai shuan shuang shui shun shuo si song sou su suan sui sun suo \
ta tai tan tang tao te teng ti tian tiao tie ting tong tou tu tuan tui tun tuo \
wa wai wan wang wei wen weng wo wu \
xi xia xian xiang xiao xie xin xing xiong xiu xu xuan xue xun \
ya yan yang yao ye yi yin ying yo yong you yu yuan yue yun \
za zai zan zang zao ze zei zen zeng zha zhai zhan zhang zhao zhe zhei zhen zheng zhi zhong zhou zhu zhua zhuai zhuan zhuang zhui zhun zhuo zi zong zou zu zuan zui zun zuo";

/// Longest legal syllable length in bytes (`zhuang`, `chuang`, `shuang` = 6).
const MAX_SYLLABLE_LEN: usize = 6;

fn syllable_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| SYLLABLE_TABLE.split_whitespace().collect())
}

/// Whether `s` is a legal toneless pinyin syllable.
pub fn is_syllable(s: &str) -> bool {
    syllable_set().contains(s)
}

/// One way to split a pinyin key sequence into syllables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyllableSplit {
    /// The syllables, in order, e.g. `["xi", "an"]`.
    pub syllables: Vec<String>,
    /// Number of input bytes covered by `syllables`. Equals the input length
    /// for a complete split; smaller when only a prefix is legal (mid-typing).
    pub consumed: usize,
}

impl SyllableSplit {
    /// Whether this split consumes the entire input.
    pub fn is_complete(&self, input_len: usize) -> bool {
        self.consumed == input_len
    }
}

/// Segment a raw pinyin key sequence into all valid syllable splits.
///
/// The input is lowercased and any explicit `'` separators (as in `xi'an`) are
/// honored as hard syllable boundaries. Results are returned best-first by a
/// simple, stable heuristic: fewer syllables first (longer syllables are
/// usually the intended reading), then lexicographic for determinism.
///
/// If no split covers the whole input (the user is still typing an incomplete
/// final syllable), the best partial split covering the longest legal prefix is
/// returned instead, so the caller always has something to look words up with.
pub fn segment(pinyin: &str) -> Vec<SyllableSplit> {
    let normalized = pinyin.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    // Honor apostrophes as hard boundaries by segmenting each run independently
    // and taking the cross product would explode; instead we treat `'` as a
    // forced cut inside the DP by only allowing a syllable to start right after
    // one. Simpler and sufficient: split on `'`, segment each piece, then join.
    if normalized.contains('\'') {
        return segment_with_boundaries(&normalized);
    }

    let bytes = normalized.as_bytes();
    let n = bytes.len();

    // Enumerate every full segmentation via DFS with memoized reachability.
    let mut full: Vec<Vec<String>> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    collect_full(&normalized, 0, &mut stack, &mut full);

    if !full.is_empty() {
        let mut splits: Vec<SyllableSplit> = full
            .into_iter()
            .map(|syllables| SyllableSplit {
                consumed: n,
                syllables,
            })
            .collect();
        sort_splits(&mut splits);
        return splits;
    }

    // No complete segmentation. Find the longest prefix that *does* fully
    // segment, so a mid-typing user still gets candidates for what they have.
    for end in (1..n).rev() {
        if !normalized.is_char_boundary(end) {
            continue;
        }
        let prefix = &normalized[..end];
        let mut partial: Vec<Vec<String>> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        collect_full(prefix, 0, &mut stack, &mut partial);
        if !partial.is_empty() {
            let mut splits: Vec<SyllableSplit> = partial
                .into_iter()
                .map(|syllables| SyllableSplit {
                    consumed: end,
                    syllables,
                })
                .collect();
            sort_splits(&mut splits);
            return splits;
        }
    }

    Vec::new()
}

/// DFS that appends to `out` every complete segmentation of `s[start..]`.
fn collect_full(s: &str, start: usize, stack: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    if start == s.len() {
        out.push(stack.clone());
        return;
    }
    let max = (start + MAX_SYLLABLE_LEN).min(s.len());
    for end in (start + 1..=max).rev() {
        if !s.is_char_boundary(end) {
            continue;
        }
        let piece = &s[start..end];
        if is_syllable(piece) {
            stack.push(piece.to_string());
            collect_full(s, end, stack, out);
            stack.pop();
        }
    }
}

/// Segment a sequence with explicit `'` boundaries by segmenting each piece and
/// concatenating the single best split of each (apostrophes remove ambiguity,
/// so keeping just the best per piece is enough and avoids a combinatorial
/// blow-up).
fn segment_with_boundaries(normalized: &str) -> Vec<SyllableSplit> {
    let mut combined: Vec<String> = Vec::new();
    let mut consumed = 0usize;
    for (index, piece) in normalized.split('\'').enumerate() {
        if index > 0 {
            consumed += 1; // the apostrophe itself
        }
        if piece.is_empty() {
            continue;
        }
        let mut sub: Vec<Vec<String>> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        collect_full(piece, 0, &mut stack, &mut sub);
        if sub.is_empty() {
            // An illegal piece aborts the explicit-boundary interpretation.
            return Vec::new();
        }
        let mut splits: Vec<SyllableSplit> = sub
            .into_iter()
            .map(|syllables| SyllableSplit {
                consumed: piece.len(),
                syllables,
            })
            .collect();
        sort_splits(&mut splits);
        combined.extend(splits.remove(0).syllables);
        consumed += piece.len();
    }
    if combined.is_empty() {
        return Vec::new();
    }
    vec![SyllableSplit {
        syllables: combined,
        consumed,
    }]
}

/// Order splits best-first: fewer syllables (longer readings) first, then
/// lexicographically for a deterministic, testable order.
fn sort_splits(splits: &mut [SyllableSplit]) {
    splits.sort_by(|a, b| {
        a.syllables
            .len()
            .cmp(&b.syllables.len())
            .then_with(|| a.syllables.cmp(&b.syllables))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_legal_syllables() {
        assert!(is_syllable("ni"));
        assert!(is_syllable("hao"));
        assert!(is_syllable("zhuang"));
        assert!(!is_syllable("xq"));
        assert!(!is_syllable(""));
    }

    #[test]
    fn splits_simple_two_syllable_input() {
        let splits = segment("nihao");
        assert!(splits.iter().any(|s| s.syllables == vec!["ni", "hao"]));
        assert!(splits.iter().all(|s| s.is_complete("nihao".len())));
    }

    #[test]
    fn enumerates_ambiguous_xian() {
        let splits = segment("xian");
        let sets: Vec<&Vec<String>> = splits.iter().map(|s| &s.syllables).collect();
        // Both the single-syllable 先 reading and the 西安 reading are legal.
        assert!(sets.iter().any(|s| **s == vec!["xian".to_string()]));
        assert!(sets
            .iter()
            .any(|s| **s == vec!["xi".to_string(), "an".to_string()]));
        // Fewer-syllable reading is offered first.
        assert_eq!(splits[0].syllables, vec!["xian"]);
    }

    #[test]
    fn honors_explicit_apostrophe_boundary() {
        let splits = segment("xi'an");
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].syllables, vec!["xi", "an"]);
    }

    #[test]
    fn returns_longest_legal_prefix_when_incomplete() {
        // `nihaox` — trailing `x` is not yet a syllable; we still segment `nihao`.
        let splits = segment("nihaox");
        assert!(!splits.is_empty());
        assert!(splits.iter().all(|s| s.consumed == "nihao".len()));
        assert!(splits.iter().any(|s| s.syllables == vec!["ni", "hao"]));
    }

    #[test]
    fn empty_input_yields_no_splits() {
        assert!(segment("").is_empty());
        assert!(segment("   ").is_empty());
    }

    #[test]
    fn full_sentence_segmentation() {
        let splits = segment("woaizhongguo");
        assert!(splits
            .iter()
            .any(|s| s.syllables == vec!["wo", "ai", "zhong", "guo"]));
    }
}
