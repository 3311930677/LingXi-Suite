//! Character-level diff between the original selection and the transformed
//! text. This is what a UI (or a console preview) shows the user before the
//! change is applied, and it stays platform independent so it is fully unit
//! testable.

/// A contiguous run of characters classified against the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp {
    /// Present in both texts, unchanged.
    Equal(String),
    /// Present only in the new text (added).
    Insert(String),
    /// Present only in the old text (removed).
    Delete(String),
}

/// Added and removed character counts for a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    pub inserted: usize,
    pub deleted: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Equal,
    Insert,
    Delete,
}

/// Compute a character-level diff using the classic longest-common-subsequence
/// dynamic program. Selections are short (a phrase or a paragraph), so the
/// O(n*m) cost is acceptable and keeps the result minimal and stable.
pub fn diff_chars(old: &str, new: &str) -> Vec<DiffOp> {
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let (n, m) = (a.len(), b.len());

    // dp[i][j] = LCS length of a[i..] and b[j..].
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut builder = OpBuilder::default();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            builder.push(Kind::Equal, a[i]);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            builder.push(Kind::Delete, a[i]);
            i += 1;
        } else {
            builder.push(Kind::Insert, b[j]);
            j += 1;
        }
    }
    while i < n {
        builder.push(Kind::Delete, a[i]);
        i += 1;
    }
    while j < m {
        builder.push(Kind::Insert, b[j]);
        j += 1;
    }
    builder.finish()
}

/// Summarize how many characters were inserted and deleted.
pub fn diff_stats(ops: &[DiffOp]) -> DiffStats {
    let mut stats = DiffStats::default();
    for op in ops {
        match op {
            DiffOp::Equal(_) => {}
            DiffOp::Insert(s) => stats.inserted += s.chars().count(),
            DiffOp::Delete(s) => stats.deleted += s.chars().count(),
        }
    }
    stats
}

/// Render a diff on one line: deletions as `[-x-]`, insertions as `[+y+]`.
pub fn render_inline(ops: &[DiffOp]) -> String {
    let mut out = String::new();
    for op in ops {
        match op {
            DiffOp::Equal(s) => out.push_str(s),
            DiffOp::Delete(s) => {
                out.push_str("[-");
                out.push_str(s);
                out.push_str("-]");
            }
            DiffOp::Insert(s) => {
                out.push_str("[+");
                out.push_str(s);
                out.push_str("+]");
            }
        }
    }
    out
}

/// Accumulates single characters into merged same-kind runs.
#[derive(Default)]
struct OpBuilder {
    ops: Vec<DiffOp>,
    current: Option<(Kind, String)>,
}

impl OpBuilder {
    fn push(&mut self, kind: Kind, ch: char) {
        match &mut self.current {
            Some((current_kind, buffer)) if *current_kind == kind => buffer.push(ch),
            _ => {
                self.flush();
                self.current = Some((kind, ch.to_string()));
            }
        }
    }

    fn flush(&mut self) {
        if let Some((kind, text)) = self.current.take() {
            self.ops.push(match kind {
                Kind::Equal => DiffOp::Equal(text),
                Kind::Insert => DiffOp::Insert(text),
                Kind::Delete => DiffOp::Delete(text),
            });
        }
    }

    fn finish(mut self) -> Vec<DiffOp> {
        self.flush();
        self.ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_all_equal() {
        assert_eq!(
            diff_chars("hello", "hello"),
            vec![DiffOp::Equal("hello".into())]
        );
    }

    #[test]
    fn pure_insertion_at_front() {
        let ops = diff_chars("world", "hi world");
        assert_eq!(
            ops,
            vec![DiffOp::Insert("hi ".into()), DiffOp::Equal("world".into())]
        );
        assert_eq!(
            diff_stats(&ops),
            DiffStats {
                inserted: 3,
                deleted: 0
            }
        );
    }

    #[test]
    fn replacement_shows_delete_then_insert() {
        let ops = diff_chars("cat", "cot");
        assert_eq!(
            ops,
            vec![
                DiffOp::Equal("c".into()),
                DiffOp::Delete("a".into()),
                DiffOp::Insert("o".into()),
                DiffOp::Equal("t".into()),
            ]
        );
        assert_eq!(render_inline(&ops), "c[-a-][+o+]t");
    }

    #[test]
    fn handles_unicode_and_emoji() {
        let ops = diff_chars("中文", "中文😀");
        assert_eq!(
            ops,
            vec![DiffOp::Equal("中文".into()), DiffOp::Insert("😀".into())]
        );
        assert_eq!(
            diff_stats(&ops),
            DiffStats {
                inserted: 1,
                deleted: 0
            }
        );
    }

    #[test]
    fn full_deletion() {
        let ops = diff_chars("abc", "");
        assert_eq!(ops, vec![DiffOp::Delete("abc".into())]);
        assert_eq!(
            diff_stats(&ops),
            DiffStats {
                inserted: 0,
                deleted: 3
            }
        );
    }
}
