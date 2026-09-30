//! Dictionary: the word ↔ pinyin ↔ frequency data the engine searches.
//!
//! This is the pure-Rust stand-in for what rime-ice provides to librime: a set
//! of entries, each mapping a Chinese word to its space-free pinyin key and a
//! frequency weight. The engine looks entries up by their exact pinyin key
//! (e.g. `nihao` → 你好); the segmenter has already decided the syllable
//! boundaries, so the dictionary only needs exact key matching plus fuzzy
//! normalization.
//!
//! For real use, [`Dictionary::load_text`] ingests a rime-ice-style table
//! (`word<TAB>pinyin<TAB>weight`, `#` comments and rime YAML front-matter
//! allowed), and [`Dictionary::load_file`] / [`Dictionary::from_files`] read
//! such tables from disk. A compact built-in dictionary
//! ([`Dictionary::builtin`]) keeps the engine usable and fully unit-testable
//! offline.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::segment::segment;

/// One dictionary record: a word, its toneless pinyin and frequency weight.
#[derive(Debug, Clone, PartialEq)]
pub struct DictionaryEntry {
    pub word: String,
    /// Pinyin syllables joined without separators, e.g. `nihao`.
    pub pinyin: String,
    /// Original syllable boundaries, e.g. `["ni", "hao"]`. They are retained
    /// so the dictionary can build a compact simplified-pinyin index (`nh`).
    pub syllables: Vec<String>,
    pub weight: u32,
}

/// A frequency-weighted pinyin dictionary with exact, fuzzy and abbreviated
/// lookup. Indexes store entry IDs rather than duplicating 500k rime-ice words.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    entries: Vec<DictionaryEntry>,
    /// Exact toneless joined pinyin → entry IDs.
    by_pinyin: HashMap<String, Vec<usize>>,
    /// Syllable initials (`ni hao` → `nh`) → entry IDs. Only multi-syllable
    /// entries are indexed; single-letter abbreviation queries are rejected.
    by_abbreviation: HashMap<String, Vec<usize>>,
    /// Whether common fuzzy-pinyin equivalences are applied on lookup.
    fuzzy: bool,
}

impl Dictionary {
    /// An empty dictionary. Fuzzy matching is on by default because Chinese
    /// users routinely rely on `zh/z`, `in/ing` tolerance.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_pinyin: HashMap::new(),
            by_abbreviation: HashMap::new(),
            fuzzy: true,
        }
    }

    /// Enable or disable fuzzy-pinyin equivalences (`zh↔z`, `ch↔c`, `sh↔s`,
    /// `in↔ing`, `en↔eng`, `l↔n`, `v↔u`). Returns `self` for chaining.
    pub fn with_fuzzy(mut self, fuzzy: bool) -> Self {
        self.fuzzy = fuzzy;
        self
    }

    /// Insert one entry and build both full-pinyin and simplified-pinyin indexes.
    /// Space/apostrophe-separated pinyin retains explicit syllable boundaries;
    /// joined pinyin is segmented automatically when possible.
    pub fn insert(&mut self, word: impl Into<String>, pinyin: impl Into<String>, weight: u32) {
        let word = word.into();
        let raw_pinyin = pinyin.into();
        let syllables = syllables_from_pinyin(&raw_pinyin, word.chars().count());
        let joined = normalize_key(&raw_pinyin);
        let abbreviation = abbreviation_key(&syllables);
        let id = self.entries.len();
        self.entries.push(DictionaryEntry {
            word,
            pinyin: joined.clone(),
            syllables,
            weight,
        });
        self.by_pinyin.entry(joined).or_default().push(id);
        if let Some(abbreviation) = abbreviation {
            self.by_abbreviation
                .entry(abbreviation)
                .or_default()
                .push(id);
        }
    }

    /// Number of distinct pinyin keys held.
    pub fn key_count(&self) -> usize {
        self.by_pinyin.len()
    }

    /// Whether the dictionary holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of loaded dictionary entries (including homophones).
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Exact lookup of all words whose pinyin key equals `key` (already
    /// syllable-joined), best-weighted first. Fuzzy matching is only attempted
    /// when no exact key exists.
    pub fn lookup(&self, key: &str) -> Vec<&DictionaryEntry> {
        let key = normalize_key(key);
        let mut ids = self.by_pinyin.get(&key).cloned().unwrap_or_default();
        if ids.is_empty() && self.fuzzy {
            let canon = fuzzy_canon(&key);
            for (candidate, bucket) in &self.by_pinyin {
                if candidate != &key && fuzzy_canon(candidate) == canon {
                    ids.extend(bucket.iter().copied());
                }
            }
        }
        self.sorted_entries(ids)
    }

    /// Lookup a pure simplified-pinyin key such as `nh` (你好) or `zgr`
    /// (中国人). A single letter is deliberately rejected to avoid enormous,
    /// noisy candidate sets.
    pub fn lookup_abbreviation(&self, key: &str) -> Vec<&DictionaryEntry> {
        let key = normalize_key(key);
        if key.len() < 2 || !key.bytes().all(|byte| byte.is_ascii_lowercase()) {
            return Vec::new();
        }
        self.sorted_entries(self.by_abbreviation.get(&key).cloned().unwrap_or_default())
    }

    fn sorted_entries(&self, mut ids: Vec<usize>) -> Vec<&DictionaryEntry> {
        ids.sort_unstable();
        ids.dedup();
        let mut entries: Vec<&DictionaryEntry> =
            ids.into_iter().map(|id| &self.entries[id]).collect();
        entries.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.word.cmp(&b.word)));
        entries
    }

    /// Load entries from a rime-ice-style text table. Each non-empty,
    /// non-comment line is `word<TAB or spaces>pinyin[<TAB or spaces>weight]`.
    /// A missing weight defaults to `1`; malformed lines are skipped. Returns
    /// the number of entries added.
    ///
    /// Real rime `.dict.yaml` files begin with a YAML front-matter block
    /// (`---` … `...`) before the tab-separated table; it is detected and
    /// skipped so such files can be ingested unchanged. The pinyin column may
    /// contain space-separated syllables (rime's `ni hao`); they are joined on
    /// insert, so both `nihao` and `ni hao` work.
    pub fn load_text(&mut self, text: &str) -> usize {
        let mut added = 0;
        let mut in_front_matter = false;
        for line in text.lines() {
            let trimmed = line.trim();
            // Skip a leading rime YAML front-matter block. `---` opens it and a
            // lone `...` (or the first tab-separated data line) closes it.
            if trimmed == "---" {
                in_front_matter = true;
                continue;
            }
            if in_front_matter {
                if trimmed == "..." {
                    in_front_matter = false;
                }
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // rime tables are tab-separated: word <TAB> pinyin <TAB> weight,
            // where pinyin may itself contain spaces ("ni hao"). Prefer tab
            // splitting; fall back to whitespace for the simpler `word pinyin`
            // form used in tests and hand-written lists.
            let (word, pinyin, weight) = if trimmed.contains('\t') {
                let mut cols = trimmed.split('\t').map(str::trim);
                let (Some(word), Some(pinyin)) = (cols.next(), cols.next()) else {
                    continue;
                };
                let weight = cols.next().and_then(|w| w.parse::<u32>().ok()).unwrap_or(1);
                (word, pinyin.to_string(), weight)
            } else {
                let mut cols = trimmed.split_whitespace();
                let (Some(word), Some(pinyin)) = (cols.next(), cols.next()) else {
                    continue;
                };
                let weight = cols.next().and_then(|w| w.parse::<u32>().ok()).unwrap_or(1);
                (word, pinyin.to_string(), weight)
            };
            if word.is_empty() || pinyin.is_empty() {
                continue;
            }
            self.insert(word, pinyin, weight);
            added += 1;
        }
        added
    }

    /// Load a rime-ice-style dictionary file from `path`, merging its entries
    /// into `self`. Returns the number of entries added. See [`load_text`] for
    /// the accepted format (including rime YAML front-matter).
    ///
    /// [`load_text`]: Dictionary::load_text
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> io::Result<usize> {
        let text = std::fs::read_to_string(path)?;
        Ok(self.load_text(&text))
    }

    /// Build a dictionary from one or more rime-ice-style files, loaded in
    /// order (later files add homophones/override nothing; weights coexist).
    /// Fuzzy matching is on by default. Fails on the first unreadable file.
    pub fn from_files<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) -> io::Result<Self> {
        let mut dict = Dictionary::new();
        for path in paths {
            dict.load_file(path)?;
        }
        Ok(dict)
    }

    /// A compact, hand-tuned built-in dictionary. It is deliberately small but
    /// covers the words used across the crate's tests and demos, so the engine
    /// produces meaningful multi-word candidates offline. Weights are relative.
    pub fn builtin() -> Self {
        let mut dict = Dictionary::new();
        // (word, pinyin, weight) — weights loosely reflect real frequency so
        // ranking behaves plausibly in tests.
        const ENTRIES: &[(&str, &str, u32)] = &[
            ("你", "ni", 9000),
            ("你好", "ni hao", 5000),
            ("好", "hao", 8000),
            ("号", "hao", 3000),
            ("你们", "ni men", 2600),
            ("我", "wo", 9500),
            ("我们", "wo men", 4200),
            ("爱", "ai", 3000),
            ("中", "zhong", 4000),
            ("中国", "zhong guo", 5200),
            ("中国人", "zhong guo ren", 4100),
            ("国", "guo", 3500),
            ("世界", "shi jie", 3800),
            ("世", "shi", 1500),
            ("界", "jie", 1200),
            ("先", "xian", 2600),
            ("西安", "xi an", 1800),
            ("现在", "xian zai", 2400),
            ("输入", "shu ru", 2200),
            ("输入法", "shu ru fa", 2600),
            ("方法", "fang fa", 2000),
            ("法", "fa", 1400),
            ("是", "shi", 9800),
            ("的", "de", 12000),
            ("测试", "ce shi", 2100),
            ("智能", "zhi neng", 2300),
            ("助手", "zhu shou", 2000),
            ("拼音", "pin yin", 2500),
            ("候选", "hou xuan", 1600),
            ("词", "ci", 1700),
            ("今天", "jin tian", 3100),
            ("天气", "tian qi", 2400),
            ("很", "hen", 5200),
            ("不错", "bu cuo", 2200),
            ("谢谢", "xie xie", 3000),
        ];
        for &(word, pinyin, weight) in ENTRIES {
            dict.insert(word, pinyin, weight);
        }
        dict
    }
}

fn syllables_from_pinyin(pinyin: &str, word_chars: usize) -> Vec<String> {
    let normalized = pinyin.trim().to_ascii_lowercase().replace('ü', "v");
    let explicit: Vec<String> = normalized
        .split(|ch: char| ch.is_whitespace() || ch == '\'')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    if explicit.len() > 1 {
        return explicit;
    }

    let joined = normalize_key(&normalized);
    let splits = segment(&joined);
    // Prefer a split matching the number of Han characters. This resolves
    // common joined dictionary entries such as `nihao` without misreading a
    // single-syllable word such as `xian`.
    splits
        .iter()
        .find(|split| split.syllables.len() == word_chars && split.consumed == joined.len())
        .or_else(|| splits.iter().find(|split| split.consumed == joined.len()))
        .map(|split| split.syllables.clone())
        .unwrap_or_else(|| vec![joined])
}

fn abbreviation_key(syllables: &[String]) -> Option<String> {
    if syllables.len() < 2 {
        return None;
    }
    syllables
        .iter()
        .map(|syllable| syllable.chars().next())
        .collect()
}

/// Normalize a pinyin key for storage/lookup: lowercased, apostrophes and
/// whitespace removed (keys are syllable-joined), `ü`→`v`.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| !c.is_whitespace() && *c != '\'')
        .map(|c| match c {
            'ü' => 'v',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Collapse a key to a fuzzy canonical form so tolerated variants compare
/// equal. Order matters: multi-char rules first. This is intentionally simple
/// and total (no panics), matching how mainstream IMEs offer opt-in fuzzy sets.
fn fuzzy_canon(key: &str) -> String {
    let mut s = key.to_string();
    // Retroflex → dental initials.
    s = s.replace("zh", "z").replace("ch", "c").replace("sh", "s");
    // Nasal finals.
    s = s
        .replace("ing", "in")
        .replace("eng", "en")
        .replace("ang", "an");
    // l/n initial confusion and ü spelling.
    s = s.replace('l', "n").replace('v', "u");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_exact_lookup_orders_by_weight() {
        let mut dict = Dictionary::new().with_fuzzy(false);
        dict.insert("号", "hao", 3000);
        dict.insert("好", "hao", 8000);
        let hits = dict.lookup("hao");
        assert_eq!(hits[0].word, "好"); // higher weight first
        assert_eq!(hits[1].word, "号");
    }

    #[test]
    fn homophones_share_a_key() {
        let dict = Dictionary::builtin();
        let hits = dict.lookup("xian");
        let words: Vec<&str> = hits.iter().map(|e| e.word.as_str()).collect();
        assert!(words.contains(&"先"));
        assert!(words.contains(&"西安"));
    }

    #[test]
    fn fuzzy_matches_retroflex_and_nasal() {
        let mut dict = Dictionary::new(); // fuzzy on
        dict.insert("中国", "zhongguo", 100);
        // Typed without the `h` and with the wrong nasal: still found via fuzzy.
        assert!(!dict.lookup("zongguo").is_empty());
    }

    #[test]
    fn fuzzy_off_is_strict() {
        let mut dict = Dictionary::new().with_fuzzy(false);
        dict.insert("中国", "zhongguo", 100);
        assert!(dict.lookup("zongguo").is_empty());
        assert!(!dict.lookup("zhongguo").is_empty());
    }

    #[test]
    fn load_text_parses_table_and_skips_junk() {
        let mut dict = Dictionary::new();
        let added = dict.load_text(
            "# comment\n你好\tnihao\t5000\n好 hao 8000\n\nmalformed_no_pinyin\n世界 shijie",
        );
        assert_eq!(added, 3); // 你好, 好, 世界 (last defaults weight 1)
        assert_eq!(dict.lookup("nihao")[0].word, "你好");
        assert_eq!(dict.lookup("shijie")[0].weight, 1);
    }

    #[test]
    fn load_text_skips_rime_yaml_front_matter() {
        let mut dict = Dictionary::new();
        let added = dict.load_text(
            "---\nname: test\nversion: \"1\"\n...\n你好\tni hao\t5000\n世界\tshi jie\t3000",
        );
        assert_eq!(added, 2);
        // Space-separated syllables in the pinyin column are joined on insert.
        assert_eq!(dict.lookup("nihao")[0].word, "你好");
        assert_eq!(dict.lookup("shijie")[0].word, "世界");
        assert_eq!(dict.lookup_abbreviation("nh")[0].word, "你好");
        assert_eq!(dict.lookup_abbreviation("sj")[0].word, "世界");
    }

    #[test]
    fn abbreviation_index_uses_syllable_initials_and_frequency() {
        let mut dict = Dictionary::new();
        dict.insert("你好", "ni hao", 5000);
        dict.insert("年号", "nian hao", 1000);
        dict.insert("中国人", "zhong guo ren", 4000);
        let nh = dict.lookup_abbreviation("nh");
        assert_eq!(nh[0].word, "你好");
        assert_eq!(nh[1].word, "年号");
        assert_eq!(dict.lookup_abbreviation("zgr")[0].word, "中国人");
        assert!(dict.lookup_abbreviation("n").is_empty());
    }

    #[test]
    fn joined_pinyin_infers_boundaries_for_abbreviation() {
        let mut dict = Dictionary::new();
        dict.insert("你好", "nihao", 5000);
        assert_eq!(dict.lookup_abbreviation("nh")[0].word, "你好");
    }

    #[test]
    fn load_file_and_from_files_read_from_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("lingxi-ime-test-{}.dict", std::process::id()));
        std::fs::write(&path, "阿\ta\t10\n爱\tai\t3000\n").expect("write temp dict");

        let dict = Dictionary::from_files([&path]).expect("load temp dict");
        assert_eq!(dict.lookup("ai")[0].word, "爱");
        assert_eq!(dict.lookup("a")[0].word, "阿");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_file_reports_missing_path_as_error() {
        let mut dict = Dictionary::new();
        assert!(dict.load_file("this/path/does/not/exist.dict").is_err());
    }

    #[test]
    fn normalize_key_strips_separators_and_lowercases() {
        assert_eq!(normalize_key("Ni'Hao"), "nihao");
        assert_eq!(normalize_key("l ü"), "lv");
    }

    #[test]
    fn builtin_is_non_empty() {
        assert!(!Dictionary::builtin().is_empty());
        assert!(Dictionary::builtin().key_count() > 20);
    }
}
