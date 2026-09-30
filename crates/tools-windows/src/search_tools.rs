//! File search tool: find files by name within the session working directory.
//!
//! Cross-platform (pure std/tokio): lives in tools-windows only because the
//! crate is the current home for all built-in tools; it has no Win32 deps.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Stop searching after this many directory visits (guards against huge trees
/// like `node_modules` / `target` and symlink-heavy layouts).
const MAX_VISITS: usize = 8_000;
/// Maximum number of matches returned to the model.
const MAX_RESULTS: usize = 100;

/// Directory names skipped during recursion: build outputs and VCS metadata
/// that would dominate the result set without ever being what the user wants.
const IGNORED_DIRS: [&str; 7] = [
    "target",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "dist",
    ".next",
];

/// Search files by case-insensitive substring match on the file name.
pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "search_files".into(),
            description: "在会话工作目录内按文件名搜索文件（子串匹配，不区分大小写）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "文件名中要匹配的子串，如 \"config\" 或 \".log\""},
                    "max_results": {"type": "integer", "description": "返回的最大结果数，默认 20，上限 100"}
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => return ToolResult::err("缺少有效的 pattern 参数"),
        };
        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS))
            .unwrap_or(20);

        // Confine the search to the session working directory, same security
        // policy as the other file tools.
        let root = match ctx.working_dir.canonicalize() {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("无法解析工作目录: {e}")),
        };

        let mut matches = Vec::new();
        let mut visited = 0usize;
        let pattern_lower = pattern.to_lowercase();

        search_recursive(
            &root,
            &pattern_lower,
            max_results,
            &mut visited,
            &mut matches,
        )
        .await;

        if matches.is_empty() {
            ToolResult::ok(format!("未找到名称包含 \"{pattern}\" 的文件"))
        } else {
            let mut output = format!("找到 {} 个匹配文件：\n", matches.len());
            for path in &matches {
                // Present paths relative to the working dir for readability.
                let display = path
                    .strip_prefix(&root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                output.push_str(&format!("- {display}\n"));
            }
            if visited >= MAX_VISITS {
                output.push_str("\n（搜索已达目录数上限，结果可能不完整）");
            }
            ToolResult::ok(output)
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

/// Depth-first recursive search. Stops early once `matches` is full or the
/// visit budget is exhausted.
async fn search_recursive(
    dir: &Path,
    pattern_lower: &str,
    max_results: usize,
    visited: &mut usize,
    matches: &mut Vec<PathBuf>,
) {
    if *visited >= MAX_VISITS || matches.len() >= max_results {
        return;
    }
    *visited += 1;

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return, // unreadable dir: skip silently
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        if matches.len() >= max_results || *visited >= MAX_VISITS {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = match entry.file_type().await {
            Ok(t) => t,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            // Do not follow symlinked directories (would risk cycles).
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            Box::pin(search_recursive(
                &path,
                pattern_lower,
                max_results,
                visited,
                matches,
            ))
            .await;
        } else if name.to_lowercase().contains(pattern_lower) {
            matches.push(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn finds_file_by_substring_case_insensitive() {
        let dir = std::env::temp_dir().join("lingxi_search_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("HelloConfig.txt"), "a").unwrap();
        fs::write(dir.join("sub/another_config.yaml"), "b").unwrap();
        fs::write(dir.join("unrelated.md"), "c").unwrap();

        let ctx = ToolContext::auto_confirm(&dir);
        let tool = SearchFilesTool;
        let result = tool.execute(json!({"pattern": "config"}), &ctx).await;

        assert!(result.success);
        assert!(result.output.contains("HelloConfig.txt"));
        assert!(result.output.contains("another_config.yaml"));
        assert!(!result.output.contains("unrelated.md"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn skips_target_directory() {
        let dir = std::env::temp_dir().join("lingxi_search_skip_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("target/findme.rs"), "x").unwrap();
        fs::write(dir.join("findme.rs"), "x").unwrap();

        let ctx = ToolContext::auto_confirm(&dir);
        let tool = SearchFilesTool;
        let result = tool.execute(json!({"pattern": "findme"}), &ctx).await;

        assert!(result.success);
        // Only the top-level file, not the one under target/.
        assert_eq!(result.output.matches("findme.rs").count(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
