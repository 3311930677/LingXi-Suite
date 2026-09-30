//! File operation tools: read, write, list directory.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::path::Path;

/// Maximum file size we are willing to read into memory (10 MB). Files larger
/// than this are rejected outright to prevent OOM on multi-GB logs or binaries.
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Read a text file's contents.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_file".into(),
            description: "读取文本文件的内容。路径可以是绝对路径或相对于工作目录的路径。文件最大 10MB，超出部分会被截断。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "文件路径"}
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("缺少 path 参数"),
        };

        let full = match resolve_existing_path(path, &ctx.working_dir) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error),
        };

        // Check the file size before reading to prevent OOM on huge files.
        // `read_to_string` / `read` would load the entire file into memory.
        let metadata = match tokio::fs::metadata(&full).await {
            Ok(m) => m,
            Err(e) => return ToolResult::err(format!("读取文件元数据失败: {e}")),
        };
        if metadata.len() > MAX_FILE_SIZE {
            return ToolResult::err(format!(
                "文件过大（{} 字节），最大支持 {} 字节。请使用其他工具处理大文件。",
                metadata.len(),
                MAX_FILE_SIZE
            ));
        }

        // Read as bytes and decode lossily so binary files don't cause a hard
        // error — `read_to_string` rejects any non-UTF-8 byte sequence.
        match tokio::fs::read(&full).await {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes);
                ToolResult::ok(truncate_utf8(&content, 50_000))
            }
            Err(e) => ToolResult::err(format!("读取文件失败: {e}")),
        }
    }
}

/// Write text to a file (creates or overwrites).
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".into(),
            description: "将文本写入文件（覆盖已有内容）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "文件路径"},
                    "content": {"type": "string", "description": "要写入的内容"}
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Dangerous
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("缺少 path 参数"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("缺少 content 参数"),
        };

        let full = match resolve_write_path(path, &ctx.working_dir) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error),
        };
        match tokio::fs::write(&full, content).await {
            Ok(()) => ToolResult::ok(format!("已写入 {}", full.display())),
            Err(e) => ToolResult::err(format!("写入文件失败: {e}")),
        }
    }
}

/// List the contents of a directory.
pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_dir".into(),
            description: "列出目录中的文件和子目录。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "目录路径（默认为工作目录）"}
                }
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let full = match resolve_existing_path(path, &ctx.working_dir) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error),
        };
        let mut entries = match tokio::fs::read_dir(&full).await {
            Ok(e) => e,
            Err(e) => return ToolResult::err(format!("读取目录失败: {e}")),
        };

        let mut items = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            items.push(json!({"name": name, "is_dir": is_dir}));
        }

        items.sort_by(|a, b| {
            let a_dir = a["is_dir"].as_bool().unwrap_or(false);
            let b_dir = b["is_dir"].as_bool().unwrap_or(false);
            b_dir.cmp(&a_dir).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            })
        });

        ToolResult::ok_with_data(format!("共 {} 项", items.len()), json!(items))
    }
}

/// Truncate a string at a valid UTF-8 boundary and add a useful notice.
fn truncate_utf8(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut end = max_bytes.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n...(文件已截断，共 {} 字节)",
        &content[..end],
        content.len()
    )
}

/// Resolve an existing path and confine it to the session working directory.
/// This prevents a model from reading credentials or system files elsewhere on
/// disk through an absolute path or `..` traversal.
fn resolve_existing_path(path: &str, working_dir: &Path) -> Result<std::path::PathBuf, String> {
    let root = working_dir
        .canonicalize()
        .map_err(|e| format!("无法解析工作目录: {e}"))?;
    let requested = if Path::new(path).is_absolute() {
        Path::new(path).to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = requested
        .canonicalize()
        .map_err(|e| format!("路径不存在或无法访问: {e}"))?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "安全策略拒绝访问工作目录之外的路径（工作目录: {}）",
            root.display()
        ));
    }
    Ok(resolved)
}

/// Resolve a write target by validating its existing parent directory.
fn resolve_write_path(path: &str, working_dir: &Path) -> Result<std::path::PathBuf, String> {
    let requested = if Path::new(path).is_absolute() {
        Path::new(path).to_path_buf()
    } else {
        working_dir.join(path)
    };
    let file_name = requested
        .file_name()
        .ok_or_else(|| "写入路径必须包含文件名".to_string())?;
    let parent = requested
        .parent()
        .ok_or_else(|| "无法解析写入路径的父目录".to_string())?;
    let safe_parent = resolve_existing_path(parent.to_string_lossy().as_ref(), working_dir)?;
    Ok(safe_parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_never_splits_multibyte_character() {
        let content = "你好吗abc";
        let truncated = truncate_utf8(content, 4);
        assert!(truncated.starts_with('你'));
        assert!(!truncated.starts_with("你好"));
        assert!(truncated.contains("文件已截断"));
    }

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
    }
}
