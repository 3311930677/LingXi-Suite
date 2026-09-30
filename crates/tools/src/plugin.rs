//! 工具插件：tool.json 清单解析与命令行执行器。
//!
//! 工具插件 = 目录 + tool.json 清单 + 脚本资源。清单声明子进程命令行，
//! Agent 调用该工具时由 `PluginTool` 拉起子进程：参数 JSON 走 stdin，
//! stdout 作为工具输出，超时自动杀死。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::context::RiskLevel;
use crate::schema::{ToolResult, ToolSchema};
use crate::Tool;

/// 清单文件名。
pub const MANIFEST_FILE: &str = "tool.json";
/// 默认执行超时（秒）。
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 最大执行超时（秒）。
const MAX_TIMEOUT_SECS: u64 = 300;

/// 工具插件清单（tool.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// 插件目录名（安装包 id，与市场条目一致）。
    pub id: String,
    /// 注册进工具注册表的工具名（LLM 调用名，全局唯一）。
    pub name: String,
    /// 市场卡片展示名；缺省回落 name。
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub author: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// safe | moderate | dangerous；缺省 moderate（第三方代码默认多一分谨慎）。
    #[serde(default = "default_risk")]
    pub risk_level: String,
    /// 子进程命令行：command[0] 必须是无路径分隔符的程序名。
    pub command: Vec<String>,
    /// 参数 JSON Schema（发给 LLM）。
    #[serde(default = "default_parameters")]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_risk() -> String {
    "moderate".into()
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

/// 解析并校验清单文本。
pub fn parse_manifest(text: &str) -> Result<PluginManifest, String> {
    let manifest: PluginManifest =
        serde_json::from_str(text).map_err(|error| format!("tool.json 格式错误：{error}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// 插件目录 id：与皮肤 id 同规则（[A-Za-z0-9_-]，≤64）。
pub fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 工具注册名：小写字母数字下划线（LLM 函数名惯例），≤64。
pub fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 校验清单：id/name 字符集、版本、命令安全性、风险级别、超时范围。
pub fn validate_manifest(manifest: &PluginManifest) -> Result<(), String> {
    if !valid_plugin_id(&manifest.id) {
        return Err(format!("插件 id 非法：{}", manifest.id));
    }
    if !valid_tool_name(&manifest.name) {
        return Err(format!("工具名非法：{}", manifest.name));
    }
    if manifest.version.trim().is_empty() {
        return Err("缺少 version".into());
    }
    let Some(program) = manifest.command.first() else {
        return Err("缺少 command".into());
    };
    if program.is_empty()
        || program.contains('/')
        || program.contains('\\')
        || Path::new(program).is_absolute()
    {
        return Err(format!("command[0] 不允许路径分隔符或绝对路径：{program}"));
    }
    match manifest.risk_level.as_str() {
        "safe" | "moderate" | "dangerous" => {}
        other => return Err(format!("risk_level 非法：{other}")),
    }
    if let Some(secs) = manifest.timeout_secs {
        if !(1..=MAX_TIMEOUT_SECS).contains(&secs) {
            return Err(format!(
                "timeout_secs 必须在 1..={MAX_TIMEOUT_SECS} 之间：{secs}"
            ));
        }
    }
    Ok(())
}

/// 工具插件执行器：清单 + 插件目录。
pub struct PluginTool {
    manifest: PluginManifest,
    dir: PathBuf,
}

impl PluginTool {
    /// 从插件目录加载（读取并校验 tool.json）。
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("读取 tool.json 失败（{}）：{error}", path.display()))?;
        let manifest = parse_manifest(&text)?;
        Ok(Self {
            manifest,
            dir: dir.to_path_buf(),
        })
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn timeout(&self) -> Duration {
        let secs = self
            .manifest
            .timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        Duration::from_secs(secs)
    }
}

#[async_trait]
impl Tool for PluginTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.manifest.name.clone(),
            description: format!(
                "[插件 {}] {}",
                self.manifest.display_name, self.manifest.description
            ),
            parameters: self.manifest.parameters.clone(),
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &crate::context::ToolContext,
    ) -> ToolResult {
        let mut command = tokio::process::Command::new(&self.manifest.command[0]);
        command
            .args(&self.manifest.command[1..])
            .current_dir(&self.dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ToolResult::err(format!(
                    "启动插件命令失败（{}）：{error}",
                    self.manifest.command[0]
                ));
            }
        };
        // 参数 JSON 写入 stdin；写完立即关闭，脚本读到 EOF 即可解析。
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let payload = params.to_string();
            if let Err(error) = stdin.write_all(payload.as_bytes()).await {
                return ToolResult::err(format!("写入插件 stdin 失败：{error}"));
            }
            let _ = stdin.shutdown().await;
        }
        // 超时：Err 时 drop 掉持有 child 的 future，kill_on_drop 自动杀进程。
        let output = match tokio::time::timeout(self.timeout(), child.wait_with_output()).await {
            Ok(result) => match result {
                Ok(output) => output,
                Err(error) => return ToolResult::err(format!("等待插件进程失败：{error}")),
            },
            Err(_) => {
                return ToolResult::err(format!("插件执行超时（{} 秒）", self.timeout().as_secs()));
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            ToolResult::ok(truncate_text(stdout.trim_end().to_string(), 64 * 1024))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            ToolResult::err(format!(
                "插件退出码 {}：{}",
                output.status.code().unwrap_or(-1),
                truncate_text(stderr.trim_end().to_string(), 2048)
            ))
        }
    }

    fn risk_level(&self) -> RiskLevel {
        match self.manifest.risk_level.as_str() {
            "dangerous" => RiskLevel::Dangerous,
            "moderate" => RiskLevel::Moderate,
            _ => RiskLevel::Safe,
        }
    }
}

/// 按 UTF-8 字符边界截断，超长时附加截断标记。
fn truncate_text(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（输出已截断）", &text[..end])
}

/// 扫描插件根目录，返回全部可加载的工具插件；单个目录失败只记日志跳过。
/// 返回具体类型 Arc<PluginTool>，调用方注册时可自动 coerce 成 Arc<dyn Tool>，
/// 并能读取 manifest（构建 id→name 映射）。
pub fn scan_plugins(root: &Path) -> Vec<std::sync::Arc<PluginTool>> {
    let mut plugins = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return plugins;
    };
    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if !valid_plugin_id(&file_name) || !entry.path().is_dir() {
            continue;
        }
        match PluginTool::load(&entry.path()) {
            Ok(plugin) => plugins.push(std::sync::Arc::new(plugin)),
            Err(error) => eprintln!("[lingxi] plugin: 跳过无效插件 {file_name}：{error}"),
        }
    }
    plugins.sort_by(|a, b| a.manifest().name.cmp(&b.manifest().name));
    plugins
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest_json(id: &str, name: &str, command: &[&str]) -> String {
        json!({
            "id": id,
            "name": name,
            "display_name": name,
            "author": "test",
            "version": "1.0.0",
            "description": "test tool",
            "command": command,
        })
        .to_string()
    }

    #[test]
    fn parse_accepts_valid_manifest_with_defaults() {
        let manifest = parse_manifest(&manifest_json(
            "demo-tool",
            "demo_upper",
            &["python", "main.py"],
        ))
        .expect("合法清单");
        assert_eq!(manifest.risk_level, "moderate");
        assert_eq!(manifest.timeout_secs, None);
        assert_eq!(manifest.parameters["type"], "object");
    }

    #[test]
    fn parse_rejects_bad_id_or_name() {
        assert!(parse_manifest(&manifest_json("../evil", "demo", &["python"])).is_err());
        assert!(parse_manifest(&manifest_json("demo", "Bad-Name", &["python"])).is_err());
        assert!(parse_manifest(&manifest_json("demo", "1demo", &["python"])).is_ok());
        assert!(parse_manifest(&manifest_json("demo", "", &["python"])).is_err());
    }

    #[test]
    fn parse_rejects_path_in_command() {
        assert!(parse_manifest(&manifest_json("demo", "demo", &["C:/evil.exe"])).is_err());
        assert!(parse_manifest(&manifest_json("demo", "demo", &["..\\evil.exe"])).is_err());
        assert!(parse_manifest(&manifest_json("demo", "demo", &[])).is_err());
    }

    #[test]
    fn parse_rejects_bad_risk_level_and_timeout() {
        let bad_risk = json!({
            "id": "demo", "name": "demo", "version": "1.0.0",
            "command": ["python"], "risk_level": "extreme",
        })
        .to_string();
        assert!(parse_manifest(&bad_risk).is_err());
        let bad_timeout = json!({
            "id": "demo", "name": "demo", "version": "1.0.0",
            "command": ["python"], "timeout_secs": 9999,
        })
        .to_string();
        assert!(parse_manifest(&bad_timeout).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn plugin_tool_runs_command_and_returns_stdout() {
        let manifest = parse_manifest(&manifest_json(
            "demo",
            "demo_echo",
            &["cmd", "/c", "echo", "hello"],
        ))
        .expect("合法清单");
        let tool = PluginTool {
            manifest,
            dir: std::env::temp_dir(),
        };
        let result = tool
            .execute(
                json!({"text": "ignored"}),
                &crate::context::ToolContext::auto_confirm("."),
            )
            .await;
        assert!(result.success, "输出：{}", result.output);
        assert_eq!(result.output, "hello");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn plugin_tool_reports_nonzero_exit() {
        let manifest = parse_manifest(&manifest_json(
            "demo",
            "demo_fail",
            &["cmd", "/c", "exit", "2"],
        ))
        .expect("合法清单");
        let tool = PluginTool {
            manifest,
            dir: std::env::temp_dir(),
        };
        let result = tool
            .execute(json!({}), &crate::context::ToolContext::auto_confirm("."))
            .await;
        assert!(!result.success);
        assert!(result.output.contains("退出码"));
    }
}
