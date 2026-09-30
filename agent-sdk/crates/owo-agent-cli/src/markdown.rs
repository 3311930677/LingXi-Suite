//! 终端 Markdown 渲染（自实现，不引第三方解析器）：
//!
//! - `MarkdownStream`：**流式**渲染器——token 逐个到达时按行缓冲，遇到换行即解析
//!   该行并输出 ANSI；代码围栏（```）内的内容整段原样保留（只加缩进与配色），
//!   避免把代码里的 `*`/`_` 当成强调标记。
//! - `strip_markdown`：去掉标记的纯文本版本（TUI transcript 用——ratatui 无法消费
//!   ANSI 字符串，需要纯文本 + 外层 Style）。
//!
//! 覆盖范围（终端场景够用）：标题、无序/有序列表、引用、分隔线、代码围栏、
//! 行内 `code`、**粗体**、[链接](url)。其余原样透传。

use colored::Colorize;

/// 流式 Markdown 渲染器（按行缓冲）。
pub(crate) struct MarkdownStream {
    buffer: String,
    in_fence: bool,
}

impl MarkdownStream {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
            in_fence: false,
        }
    }

    /// 追加增量，返回此刻可安全输出的文本（未成行的尾行留在缓冲里）。
    pub(crate) fn push(&mut self, delta: &str) -> String {
        self.buffer.push_str(delta);
        let mut out = String::new();
        while let Some(index) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=index).collect();
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            out.push_str(&self.render_line(&line));
            out.push('\n');
        }
        out
    }

    /// 收尾：冲刷未换行的尾行（回合结束时调用），必要时关闭围栏。
    pub(crate) fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            out.push_str(&self.render_line(&line));
        }
        if self.in_fence {
            self.in_fence = false;
            out.push_str(&format!("\n{}", "└─ 代码块结束 ─".dimmed()));
        }
        out
    }

    fn render_line(&mut self, line: &str) -> String {
        let trimmed = line.trim_end();
        let leading = trimmed.trim_start();
        if let Some(fence) = leading.strip_prefix("```") {
            if self.in_fence {
                self.in_fence = false;
                return "└─ 代码块结束 ─".dimmed().to_string();
            }
            self.in_fence = true;
            let lang = fence.trim();
            return if lang.is_empty() {
                "┌─ 代码 ─".dimmed().to_string()
            } else {
                format!("┌─ {lang} ─").dimmed().to_string()
            };
        }
        if self.in_fence {
            // 代码行：缩进 + 青色，不做行内解析。
            return format!("  {}", line.cyan());
        }
        render_inline_line(trimmed)
    }
}

/// 单行（非代码块内）渲染。
fn render_inline_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return String::new();
    }
    // 标题：# ~ ######
    let hashes = trimmed.chars().take_while(|character| *character == '#').count();
    if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
        let body = trimmed[hashes..].trim();
        return render_inline(body).bold().to_string();
    }
    // 分隔线
    if matches!(trimmed, "---" | "***" | "___") {
        return "────────────".dimmed().to_string();
    }
    // 引用
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return format!("{} {}", "│".dimmed(), render_inline(rest).dimmed());
    }
    // 无序列表
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return format!("  {} {}", "•".dimmed(), render_inline(rest));
    }
    // 有序列表
    if let Some((number, rest)) = split_ordered_item(trimmed) {
        return format!("  {}. {}", number.bold(), render_inline(rest));
    }
    render_inline(trimmed)
}

/// `"12. 内容"` → `Some(("12", "内容"))`。
fn split_ordered_item(line: &str) -> Option<(&str, &str)> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = line[digits..].strip_prefix(". ")?;
    Some((&line[..digits], rest))
}

/// 行内渲染：`**粗体**`、`` `代码` ``、`[文本](url)`，其余原样。
fn render_inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                out.push_str(&stripped[..end].bold().to_string());
                rest = &stripped[end + 2..];
                continue;
            }
        }
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                out.push_str(&stripped[..end].cyan().to_string());
                rest = &stripped[end + 1..];
                continue;
            }
        }
        if let Some(stripped) = rest.strip_prefix('[') {
            if let Some(close) = stripped.find("](") {
                let label = &stripped[..close];
                let after = &stripped[close + 2..];
                if let Some(paren) = after.find(')') {
                    let url = &after[..paren];
                    out.push_str(&label.cyan().to_string());
                    out.push_str(&format!(" ({})", url.dimmed()));
                    rest = &after[paren + 1..];
                    continue;
                }
            }
        }
        // 普通字符：逐个推进（标记前缀已在上面处理）。
        let first_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&rest[..first_len]);
        rest = &rest[first_len..];
    }
    out
}

/// 去标记纯文本（TUI transcript 用）：代码块内容保留、行内标记剥掉。
pub(crate) fn strip_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push_str("  ");
            out.push_str(line.trim_end());
            out.push('\n');
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        let body = if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
            trimmed[hashes..].trim()
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out.push_str("  • ");
            rest
        } else {
            trimmed
        };
        out.push_str(&strip_inline(body));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn strip_inline(text: &str) -> String {
    let mut out = text.replace("**", "").replace('`', "");
    // 链接：[文本](url) → 文本 (url)
    while let Some(open) = out.find('[') {
        let Some(close_rel) = out[open..].find("](") else {
            break;
        };
        let close = open + close_rel;
        let Some(paren_rel) = out[close + 2..].find(')') else {
            break;
        };
        let paren = close + 2 + paren_rel;
        let label = out[open + 1..close].to_string();
        let url = out[close + 2..paren].to_string();
        out.replace_range(open..=paren, &format!("{label} ({url})"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str) -> String {
        let mut stream = MarkdownStream::new();
        let mut out = stream.push(text);
        out.push_str(&stream.finish());
        out
    }

    #[test]
    fn headings_and_lists_get_markers() {
        let rendered = render("# 标题\n- 项目一\n1. 步骤一\n");
        assert!(rendered.contains("标题"), "{rendered}");
        assert!(rendered.contains('•'), "{rendered}");
        assert!(rendered.contains("1."), "{rendered}");
        // 标题不再残留 # 号。
        assert!(!rendered.contains("# "), "{rendered}");
    }

    #[test]
    fn code_fence_content_is_untouched() {
        let rendered = render("说明\n```rust\nlet x = **a** * b;\n```\n结束\n");
        assert!(rendered.contains("let x = **a** * b;"), "代码内容不得被解析：{rendered}");
        assert!(rendered.contains("结束"), "{rendered}");
    }

    #[test]
    fn streaming_split_across_deltas() {
        // token 边界落在行内/标记中间时也要正确拼接。
        let mut stream = MarkdownStream::new();
        let mut out = stream.push("**粗");
        out.push_str(&stream.push("体**\n"));
        out.push_str(&stream.finish());
        assert!(out.contains("粗体"), "{out}");
        assert!(!out.contains("**"), "标记应被消耗：{out}");
    }

    #[test]
    fn strip_removes_inline_markers_and_keeps_code() {
        let stripped = strip_markdown("# 标题\n**粗体** 与 `代码`\n```sh\nls -la\n```\n");
        assert!(!stripped.contains('#'), "{stripped}");
        assert!(!stripped.contains('*'), "{stripped}");
        assert!(stripped.contains("粗体") && stripped.contains("代码"), "{stripped}");
        assert!(stripped.contains("ls -la"), "代码内容保留：{stripped}");
    }
}
