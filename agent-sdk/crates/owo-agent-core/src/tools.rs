use crate::audit::AuditLog;
use crate::mcp::{McpClient, McpPrompt, McpResource, McpTool};
use crate::permissions::Policy;
use crate::session::Session;
use crate::skill::SkillRegistry;
use crate::subagent::SubagentRunner;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct ToolContext<'a> {
    pub workspace: &'a Path,
    pub policy: &'a Policy,
    pub session: &'a mut Session,
    pub audit: &'a Arc<Mutex<AuditLog>>,
    pub subagent: Option<SubagentRunner<'a>>,
    pub skills: &'a SkillRegistry,
    /// 窗口元素注册表（感知多源融合的稳定元素 ID 空间）。
    pub elements: &'a Arc<Mutex<crate::ElementRegistry>>,
    /// 用户提问通道（ask_user 工具）：None 表示当前环境没有 UI 通道（CLI/子代理），
    /// 工具会明确报错并提示模型改为书面提问。
    pub questioner: Option<&'a dyn crate::question::Questioner>,
    /// fan-out 并行子代理通道（A5-1）：重任务主会话注入；None（CLI/子代理/测试）
    /// 时 `fan_out_subagents` 工具明确报错。
    pub fanout: Option<crate::subagent::FanOutRunner>,
    /// 主回合中止标志：fan-out 工具桥接为主会话 → 子代理的取消传播。
    pub abort: Option<&'a std::sync::atomic::AtomicBool>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String>;

    /// 支持并发执行的只读工具返回 `Some(self)`；默认 `None` → 走串行 `ToolContext` 路径。
    /// 并行入口不持有 `&mut Session`，所以写类工具（需写快照）永远返回 None。
    fn as_read_only(&self) -> Option<&(dyn ReadOnlyTool + Send + Sync)> {
        None
    }
}

/// 只读工具：不写会话状态，仅依赖工作区与策略，可被同一轮的多个调用并发执行。
#[async_trait]
pub trait ReadOnlyTool: Send + Sync {
    async fn run_read_only(
        &self,
        workspace: &Path,
        policy: &Policy,
        args: Value,
    ) -> Result<Value, String>;
}

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
    /// MCP 大 schema 的完整副本（注册时超预算被压缩为骨架；此处保留原始 schema 供按需查询）。
    full_schemas: HashMap<String, Value>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            tools: Vec::new(),
            full_schemas: HashMap::new(),
        };
        registry.register(ReadFileTool);
        registry.register(WriteFileTool);
        registry.register(EditFileTool);
        registry.register(MultiEditTool);
        registry.register(ListDirTool);
        registry.register(SearchFilesTool);
        registry.register(GrepTool);
        registry.register(RunCommandTool);
        registry.register(ExploreTool);
        registry.register(SubagentTool);
        registry.register(FanOutSubagentsTool);
        registry.register(UseSkillTool);
        registry.register(AskUserTool);
        // 批次 5：代码工程能力（web API 化 / git 只读包装）。
        registry.register(crate::web_tools::WebSearchTool);
        registry.register(crate::web_tools::WebFetchTool);
        registry.register(crate::git_tools::GitStatusTool);
        registry.register(crate::git_tools::GitDiffTool);
        registry.register(crate::git_tools::GitLogTool);
        registry.register(crate::plan_tools::UpdatePlanTool);
        registry.register(crate::computer_use::ScreenOcrTool);
        registry.register(crate::computer_use::OcrRegionTool);
        registry.register(crate::computer_use::DesktopWindowOcrTool);
        registry.register(crate::computer_use::DesktopForegroundTool);
        registry.register(crate::computer_use::DesktopWindowListTool);
        registry.register(crate::computer_use::DesktopActivateTool);
        registry.register(crate::computer_use::DesktopClickTool);
        registry.register(crate::computer_use::DesktopTypeTool);
        registry.register(crate::computer_use::DesktopKeyTool);
        registry.register(crate::computer_use::DesktopShortcutTool);
        registry.register(crate::computer_use::DesktopLaunchTool);
        registry.register(crate::computer_use::DesktopScrollTool);
        registry.register(crate::computer_use::DesktopWaitTool);
        registry.register(crate::computer_use::DesktopWaitUntilTool);
        registry.register(crate::computer_use::ScreenVisionTool);
        registry.register(crate::computer_use::VisionVerifyTool);
        registry.register(crate::computer_use::VisionGroundTool);
        let browser = crate::computer_use::BrowserTools::new();
        registry.register(crate::computer_use::BrowserNavigateTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserSearchTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserSnapshotTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserClickTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserTypeTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserPressTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserScreenshotWriteTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserDownloadImageWriteTool {
            tools: browser.clone(),
        });
        registry.register(crate::computer_use::BrowserCloseTool { tools: browser });
        registry
    }

    /// 只读工具表（子代理 explore 使用）：不含写/执行/委派工具。
    pub fn read_only() -> Self {
        let mut registry = Self {
            tools: Vec::new(),
            full_schemas: HashMap::new(),
        };
        registry.register(ReadFileTool);
        registry.register(ListDirTool);
        registry.register(SearchFilesTool);
        registry.register(GrepTool);
        // git 只读三件套（A3-4）：子代理定位/评审也需要仓库历史与变更视图。
        registry.register(crate::git_tools::GitStatusTool);
        registry.register(crate::git_tools::GitDiffTool);
        registry.register(crate::git_tools::GitLogTool);
        registry
    }

    /// 无桌面工具集（评测与无 UI 环境）：文件读写/检索 + 命令 + explore 只读子代理。
    ///
    /// 刻意不含 `screen_*` / `desktop_*` / `vision_*` / `browser_*` 与通用 `subagent`
    /// —— 前者会真实操作本机桌面/浏览器，后者会另建整表、间接可达这些工具，
    /// 在自动化评测里都属副作用。
    pub fn headless() -> Self {
        let mut registry = Self {
            tools: Vec::new(),
            full_schemas: HashMap::new(),
        };
        registry.register(ReadFileTool);
        registry.register(WriteFileTool);
        registry.register(EditFileTool);
        registry.register(MultiEditTool);
        registry.register(ListDirTool);
        registry.register(SearchFilesTool);
        registry.register(GrepTool);
        registry.register(RunCommandTool);
        registry.register(ExploreTool);
        registry.register(crate::plan_tools::UpdatePlanTool);
        // 评测也覆盖 web/git 能力（无桌面依赖）。
        registry.register(crate::web_tools::WebSearchTool);
        registry.register(crate::web_tools::WebFetchTool);
        registry.register(crate::git_tools::GitStatusTool);
        registry.register(crate::git_tools::GitDiffTool);
        registry.register(crate::git_tools::GitLogTool);
        registry
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.push(Arc::new(tool));
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|tool| tool.spec()).collect()
    }

    /// 按前缀撤销工具（插件热卸载：`owo_plugin_<id>_` 前缀）。
    /// 返回被移除的工具数。
    pub fn remove_prefix(&mut self, prefix: &str) -> usize {
        self.remove_prefix_inner(prefix)
    }

    /// 取工具句柄（Arc 克隆，锁外可跨 await 执行）。
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.spec().name == name)
            .cloned()
    }

    pub async fn execute(
        &self,
        name: &str,
        ctx: &mut ToolContext<'_>,
        args: Value,
    ) -> Result<Value, String> {
        let tool = self.get(name).ok_or_else(|| format!("未知工具：{name}"))?;
        tool.run(ctx, args).await
    }

    /// 把 MCP 服务器暴露的工具注册为 Agent 工具（命名：`{server}_{tool}`）。
    ///
    /// 延迟加载（M2）：单工具 schema 超过 `schema_budget_bytes` 时，注册为模型可见的
    /// **压缩骨架**（仅保留 type/required/属性名+属性类型，剔除 description/enum/嵌套细节），
    /// 完整 schema 保留在 `full_schemas` 供 `full_schema()` 按需查询——大 schema 服务不
    /// 显著占用模型上下文；调用工具时仍以完整 schema 校验。
    pub fn register_mcp_tools(
        &mut self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<McpClient>>,
        tools: Vec<McpTool>,
    ) {
        let budget = schema_budget_bytes();
        for tool in tools {
            let full_name = format!(
                "{}_{}",
                sanitize_tool_name(server_name),
                sanitize_tool_name(&tool.name)
            );
            let (input_schema, full_schema) = if schema_bytes(&tool.input_schema) > budget {
                let full = tool.input_schema.clone();
                (compact_schema(&tool.input_schema), Some(full))
            } else {
                (tool.input_schema, None)
            };
            let mut description = tool.description;
            if full_schema.is_some() {
                description.push_str("（schema 已压缩，完整参数见 /mcp/schema 接口）");
            }
            let spec = ToolSpec {
                name: full_name.clone(),
                description,
                input_schema,
            };
            if let Some(full) = full_schema {
                self.full_schemas.insert(full_name.clone(), full);
            }
            self.tools.push(Arc::new(McpToolAdapter {
                full_name,
                server_name: server_name.to_string(),
                tool_name: tool.name,
                spec,
                client: Arc::clone(&client),
            }));
        }
    }

    /// A2-2：把 MCP 服务器的 resources/prompts 注册为**泛化工具**——
    /// 每服务器至多 2 个（`{server}_read_resource` / `{server}_get_prompt`），
    /// 逐资源/逐模板开工具会撑爆工具表；目录摘要放在描述里供模型选 URI/模板名。
    pub fn register_mcp_extras(
        &mut self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<McpClient>>,
        resources: Vec<McpResource>,
        prompts: Vec<McpPrompt>,
    ) {
        let prefix = sanitize_tool_name(server_name);
        if !resources.is_empty() {
            let catalog: Vec<String> = resources
                .iter()
                .take(12)
                .map(|resource| {
                    if resource.name.is_empty() {
                        resource.uri.clone()
                    } else {
                        format!("{}（{}）", resource.uri, resource.name)
                    }
                })
                .collect();
            let full_name = format!("{prefix}_read_resource");
            self.tools.push(Arc::new(McpResourceAdapter {
                full_name: full_name.clone(),
                server_name: server_name.to_string(),
                spec: ToolSpec {
                    name: full_name,
                    description: format!(
                        "读取 MCP 服务器 {server_name} 的资源内容（resources/read）。可用资源：{}",
                        catalog.join("；")
                    ),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "uri": { "type": "string", "description": "资源 URI（见工具描述里的目录）" }
                        },
                        "required": ["uri"]
                    }),
                },
                client: Arc::clone(&client),
            }));
        }
        if !prompts.is_empty() {
            let catalog: Vec<String> = prompts
                .iter()
                .take(12)
                .map(|prompt| prompt.name.clone())
                .collect();
            let full_name = format!("{prefix}_get_prompt");
            self.tools.push(Arc::new(McpPromptAdapter {
                full_name: full_name.clone(),
                server_name: server_name.to_string(),
                spec: ToolSpec {
                    name: full_name,
                    description: format!(
                        "获取 MCP 服务器 {server_name} 的提示模板（prompts/get）。可用模板：{}",
                        catalog.join("；")
                    ),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "模板名" },
                            "arguments": { "type": "object", "description": "模板参数（可选，键值对）" }
                        },
                        "required": ["name"]
                    }),
                },
                client,
            }));
        }
    }

    /// 按需取 MCP 工具的完整 schema（压缩注册时保留；小 schema 工具不重复存储）。
    pub fn full_schema(&self, name: &str) -> Option<Value> {
        self.full_schemas.get(name).cloned()
    }

    /// 移除工具时同步清理完整 schema 副本。
    fn remove_prefix_inner(&mut self, prefix: &str) -> usize {
        let before = self.tools.len();
        self.tools
            .retain(|tool| !tool.spec().name.starts_with(prefix));
        self.full_schemas
            .retain(|name, _| !name.starts_with(prefix));
        before - self.tools.len()
    }
}

/// 工具名只允许字母数字、下划线与连字符（模型 API 约束）。
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// MCP 工具注册前缀（`{server}_{tool}` 命名空间）：如 `owo_plugin_owo-translate_`。
pub fn mcp_tool_prefix(server_name: &str) -> String {
    format!("{}_", sanitize_tool_name(server_name))
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn required_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("参数缺少字符串字段：{key}"))
}

fn snapshot_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 递归遍历时跳过的目录（VCS 元数据、构建产物、依赖缓存）。
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
    ".owo-attachments",
    ".vscode",
    ".idea",
];

/// read_file 愿意载入的最大文件（10MB），防止 OOM。
const MAX_READABLE_FILE_SIZE: u64 = 10 * 1024 * 1024;
/// read_file 默认读取行数。
const DEFAULT_READ_LIMIT: usize = 2000;
/// read_file 单次输出的字符上限（行号格式化后）。
const MAX_READ_CHARS: usize = 50_000;
/// 二进制嗅探窗口（字节）。
const BINARY_SNIFF_WINDOW: usize = 8 * 1024;

/// 文件是否疑似二进制：嗅探窗口内出现 NUL 字节即判定。
fn looks_binary(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .take(BINARY_SNIFF_WINDOW)
        .any(|&byte| byte == 0)
}

/// 按字符数截断（UTF-8 安全），超出时补省略号。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut truncated: String = text.chars().take(max_chars).collect();
        truncated.push('…');
        truncated
    }
}

/// 简单文件名通配：支持 `*.ext` 后缀过滤与完整字面名（大小写不敏感）。
fn glob_matches(glob: &str, name: &str) -> bool {
    if let Some(ext) = glob.strip_prefix("*.") {
        name.rsplit('.')
            .next()
            .map(|found| found.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
    } else {
        name.eq_ignore_ascii_case(glob)
    }
}

/// MCP 工具 schema 延迟加载（M2）：单工具 schema 序列化字节数预算，默认 2048 字节。
pub fn schema_budget_bytes() -> usize {
    std::env::var("OWO_MCP_SCHEMA_BUDGET_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2048)
}

/// 估算 JSON schema 的序列化体积（字节）。
pub fn schema_bytes(schema: &Value) -> usize {
    serde_json::to_string(schema)
        .map(|text| text.len())
        .unwrap_or(usize::MAX)
}

/// 把 JSON Schema 压缩为模型可见的骨架：仅保留 `type`、`required` 与
/// 属性名+属性类型（字符串属性类型；嵌套对象/数组仅保留层级 type）。
/// 剔除 description / enum / pattern / 嵌套细节，体积大幅缩小。
/// 非对象 schema 原样返回（按需加载不适用）。
pub fn compact_schema(schema: &Value) -> Value {
    let Value::Object(map) = schema else {
        return schema.clone();
    };
    let mut compact = serde_json::Map::new();
    if let Some(t) = map.get("type") {
        compact.insert("type".to_string(), t.clone());
    }
    if let Some(required) = map.get("required") {
        compact.insert("required".to_string(), required.clone());
    }
    if let Some(properties) = map.get("properties").and_then(Value::as_object) {
        let mut props = serde_json::Map::new();
        for (name, property) in properties {
            let mut item = serde_json::Map::new();
            if let Some(t) = property.get("type") {
                item.insert("type".to_string(), t.clone());
            }
            props.insert(name.clone(), Value::Object(item));
        }
        compact.insert("properties".to_string(), Value::Object(props));
    }
    Value::Object(compact)
}

/// 以工作区为基座解析相对路径，并做策略工作区越界检查（不依赖 ToolContext）。
pub(crate) fn resolve_path_within(
    workspace: &Path,
    policy: &Policy,
    path: &str,
) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        workspace.join(raw)
    };
    // 不能用裸 starts_with：Windows 下 canonicalize 结果带 `\\?\` 前缀，
    // 与未 canonicalize 的原始路径形态不一致会误报越界（新建文件/显式 cwd 场景实测）。
    if !crate::permissions::path_within(&candidate, policy.workspace()) {
        return Err(format!("路径越界：{path}"));
    }
    Ok(candidate.canonicalize().unwrap_or(candidate))
}

pub(crate) fn resolve_session_path(ctx: &ToolContext, path: &str) -> Result<PathBuf, String> {
    resolve_path_within(ctx.workspace, ctx.policy, path)
}

struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "读取工作区内的文本文件，返回带行号的内容（cat -n 风格）。长文件用 offset/limit 分页读取；修改文件前必须先读到目标区段。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件路径（相对工作区或绝对）" },
                    "offset": { "type": "integer", "description": "起始行号（1-based，默认 1）" },
                    "limit": { "type": "integer", "description": "读取行数（默认 2000）" }
                },
                "required": ["path"]
            }),
        }
    }

    fn as_read_only(&self) -> Option<&(dyn ReadOnlyTool + Send + Sync)> {
        Some(self)
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        read_file_impl(ctx.workspace, ctx.policy, args).await
    }
}

/// read_file 共享实现（串行 ToolContext 与并行只读入口共用）。
async fn read_file_impl(workspace: &Path, policy: &Policy, args: Value) -> Result<Value, String> {
    let path = required_string(&args, "path")?;
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .map(|value| value.max(1) as usize)
        .unwrap_or(1);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| (value.max(1) as usize).min(5_000))
        .unwrap_or(DEFAULT_READ_LIMIT);
    let abs = resolve_path_within(workspace, policy, &path)?;

    let metadata = tokio::fs::metadata(&abs)
        .await
        .map_err(|e| format!("读取 {path} 元数据失败：{e}"))?;
    if !metadata.is_file() {
        return Err(format!("{path} 不是文件（如为目录请用 list_dir）"));
    }
    if metadata.len() > MAX_READABLE_FILE_SIZE {
        return Err(format!(
            "文件过大（{} 字节 > {}）；请用 grep 定位目标行后用 offset 分页读取",
            metadata.len(),
            MAX_READABLE_FILE_SIZE
        ));
    }
    let bytes = tokio::fs::read(&abs)
        .await
        .map_err(|e| format!("读取 {path} 失败：{e}"))?;
    if looks_binary(&bytes) {
        return Err(format!("{path} 疑似二进制文件，拒绝读取"));
    }
    let content = String::from_utf8_lossy(&bytes);
    let total_lines = content.lines().count();
    if offset > total_lines {
        return Ok(json!({
            "path": path,
            "total_lines": total_lines,
            "start_line": 0,
            "end_line": 0,
            "truncated": false,
            "content": format!("[offset={offset} 超出文件范围：共 {total_lines} 行]"),
        }));
    }

    let start_line = offset;
    let mut end_line = (start_line + limit - 1).min(total_lines);
    let mut rendered = String::new();
    let mut chars_used = 0usize;
    let mut cut_by_chars = false;
    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        if line_number < start_line {
            continue;
        }
        if line_number > end_line {
            break;
        }
        let formatted = format!("{line_number:>6}\t{line}\n");
        if chars_used + formatted.chars().count() > MAX_READ_CHARS {
            end_line = line_number.saturating_sub(1).max(start_line);
            cut_by_chars = true;
            break;
        }
        rendered.push_str(&formatted);
        chars_used += formatted.chars().count();
    }
    let truncated = cut_by_chars || end_line < total_lines;
    if truncated {
        rendered.push_str(&format!(
                "[已截断：显示第 {start_line}-{end_line} 行 / 共 {total_lines} 行；继续读取用 offset={}]\n",
                end_line + 1
            ));
    }
    Ok(json!({
        "path": path,
        "total_lines": total_lines,
        "start_line": start_line,
        "end_line": end_line,
        "truncated": truncated,
        "content": rendered,
    }))
}

#[async_trait]
impl ReadOnlyTool for ReadFileTool {
    async fn run_read_only(
        &self,
        workspace: &Path,
        policy: &Policy,
        args: Value,
    ) -> Result<Value, String> {
        read_file_impl(workspace, policy, args).await
    }
}

struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "新建文件，或改动极大时的整文件重写（覆盖已有内容）。对已有文件做局部修改优先用 edit_file。自动快照，可 diff/revert。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let content = required_string(&args, "content")?;
        let abs = resolve_session_path(ctx, &path)?;
        ensure_snapshot(ctx, &abs).await?;
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建目录失败：{e}"))?;
        }
        tokio::fs::write(&abs, content.as_bytes())
            .await
            .map_err(|e| format!("写入 {path} 失败：{e}"))?;
        Ok(json!({
            "path": path,
            "written": true,
            "bytes": content.len(),
        }))
    }
}

/// 写入前确保快照存在（每文件仅首次写入时记录原始内容，供 diff/revert）。
async fn ensure_snapshot(ctx: &mut ToolContext<'_>, abs: &Path) -> Result<(), String> {
    let key = snapshot_key(abs);
    // 回合归属：此刻 messages 只有历史（本回合消息在回合末才 commit），
    // 其长度即本回合用户消息将落到的下标，供 /undo 精确回滚本回合的写操作。
    let turn = ctx.session.messages.len();
    if let std::collections::hash_map::Entry::Vacant(entry) = ctx.session.snapshots.entry(key) {
        let original = match tokio::fs::read(abs).await {
            Ok(bytes) => Some(BASE64.encode(bytes)),
            Err(_) => None,
        };
        entry.insert(crate::session::SnapshotEntry {
            original_b64: original,
            turn,
        });
    }
    Ok(())
}

struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".into(),
            description: "对已有文件做局部精准替换。old_str 必须先用 read_file 确认且与文件内容完全一致（含缩进与空行）、在文件中唯一；多处相同需扩大上下文或设 replace_all。新建文件用 write_file。自动快照，可 diff/revert。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件路径" },
                    "old_str": { "type": "string", "description": "要替换的原文（必须唯一、含缩进）" },
                    "new_str": { "type": "string", "description": "替换后的内容" },
                    "replace_all": { "type": "boolean", "description": "替换全部出现位置（默认 false）" }
                },
                "required": ["path", "old_str", "new_str"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let old_str = required_string(&args, "old_str")?;
        let new_str = required_string(&args, "new_str")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if old_str.is_empty() {
            return Err("old_str 不能为空（新建文件请用 write_file）".to_string());
        }
        let abs = resolve_session_path(ctx, &path)?;
        if !abs.is_file() {
            return Err(format!("{path} 不存在（新建文件请用 write_file）"));
        }

        let original = match tokio::fs::read(&abs).await {
            Ok(bytes) => String::from_utf8(bytes)
                .map_err(|_| format!("{path} 不是 UTF-8 文本文件，暂不支持精准编辑"))?,
            Err(e) => return Err(format!("读取 {path} 失败：{e}")),
        };
        let occurrences = original.matches(&old_str).count();
        if occurrences == 0 {
            return Err(
                "未找到 old_str。请先 read_file 核对目标区段（注意首尾空白与缩进），用完整且唯一的片段重试"
                    .to_string(),
            );
        }
        if occurrences > 1 && !replace_all {
            return Err(format!(
                "old_str 出现 {occurrences} 次。请扩大上下文使其唯一，或设 replace_all=true"
            ));
        }

        ensure_snapshot(ctx, &abs).await?;
        let updated = if replace_all {
            original.replace(&old_str, &new_str)
        } else {
            original.replacen(&old_str, &new_str, 1)
        };
        tokio::fs::write(&abs, updated.as_bytes())
            .await
            .map_err(|e| format!("写入 {path} 失败：{e}"))?;

        // 返回第 1 处修改点的上下文预览（模型自校验）。
        let preview = locate_edit_preview(&updated, &new_str);
        Ok(json!({
            "path": path,
            "replaced": occurrences,
            "preview": preview,
        }))
    }
}

/// 找到 new_str 首次出现位置，返回其前后各 3 行（带行号）。
fn locate_edit_preview(updated: &str, new_str: &str) -> String {
    let Some(byte_index) = updated.find(new_str) else {
        return String::new();
    };
    let target_line = updated[..byte_index].lines().count() + 1;
    let start = target_line.saturating_sub(3);
    let mut rendered = String::new();
    for (index, line) in updated.lines().enumerate().skip(start).take(7) {
        rendered.push_str(&format!(
            "{:>6}\t{}\n",
            index + 1,
            truncate_chars(line, 200)
        ));
    }
    rendered
}

struct MultiEditTool;

/// A3-2 批量编辑：对同一文件顺序应用多个替换，**原子生效**——
/// 任一处匹配失败则整体不落盘（避免半成品状态逼模型手工收拾）。
#[async_trait]
impl Tool for MultiEditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "multi_edit".into(),
            description: "对同一文件做多处精准替换（一次调用替代多次 edit_file，省回合）。edits 按顺序应用——后面替换的 old_str 要匹配前面替换后的内容。原子性：任一处失败则整批不生效。old_str 必须先 read_file 确认且唯一（或设 replace_all）。自动快照，可 diff/revert。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件路径" },
                    "edits": {
                        "type": "array",
                        "description": "替换列表（按顺序应用，上限 20 个）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_str": { "type": "string", "description": "要替换的原文（必须唯一、含缩进）" },
                                "new_str": { "type": "string", "description": "替换后的内容" },
                                "replace_all": { "type": "boolean", "description": "替换该片段的全部出现位置（默认 false）" }
                            },
                            "required": ["old_str", "new_str"]
                        }
                    }
                },
                "required": ["path", "edits"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let raw_edits = args
            .get("edits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if raw_edits.is_empty() {
            return Err("edits 不能为空".to_string());
        }
        if raw_edits.len() > 20 {
            return Err(format!(
                "edits 过多（{} 个 > 20）。请拆分为多次 multi_edit 调用",
                raw_edits.len()
            ));
        }
        // 预解析全部替换项（先整体校验参数，再动文件）。
        let mut planned: Vec<(String, String, bool)> = Vec::with_capacity(raw_edits.len());
        for (index, edit) in raw_edits.iter().enumerate() {
            let old_str = edit
                .get("old_str")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}] 缺少 old_str"))?
                .to_string();
            let new_str = edit
                .get("new_str")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}] 缺少 new_str"))?
                .to_string();
            if old_str.is_empty() {
                return Err(format!("edits[{index}] 的 old_str 不能为空"));
            }
            let replace_all = edit
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            planned.push((old_str, new_str, replace_all));
        }

        let abs = resolve_session_path(ctx, &path)?;
        if !abs.is_file() {
            return Err(format!("{path} 不存在（新建文件请用 write_file）"));
        }
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|e| format!("读取 {path} 失败：{e}"))?;
        let mut content = String::from_utf8(bytes)
            .map_err(|_| format!("{path} 不是 UTF-8 文本文件，暂不支持精准编辑"))?;

        // 内存中顺序应用；任一处失败立即返回，原文件不受影响。
        for (index, (old_str, new_str, replace_all)) in planned.iter().enumerate() {
            let occurrences = content.matches(old_str.as_str()).count();
            if occurrences == 0 {
                return Err(format!(
                    "第 {}/{} 处替换失败：未找到 old_str（前面的替换可能已改变上下文）。整批未应用，请 read_file 后重试",
                    index + 1,
                    planned.len()
                ));
            }
            if occurrences > 1 && !replace_all {
                return Err(format!(
                    "第 {}/{} 处替换失败：old_str 出现 {occurrences} 次。请扩大上下文使其唯一或设 replace_all。整批未应用",
                    index + 1,
                    planned.len()
                ));
            }
            content = if *replace_all {
                content.replace(old_str.as_str(), new_str)
            } else {
                content.replacen(old_str.as_str(), new_str, 1)
            };
        }

        ensure_snapshot(ctx, &abs).await?;
        tokio::fs::write(&abs, content.as_bytes())
            .await
            .map_err(|e| format!("写入 {path} 失败：{e}"))?;

        // 预览最后一处修改点（模型自校验）。
        let last_new = planned.last().map(|(_, new, _)| new.clone()).unwrap_or_default();
        let preview = locate_edit_preview(&content, &last_new);
        Ok(json!({
            "path": path,
            "applied": planned.len(),
            "preview": preview,
        }))
    }
}

struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "列出工作区内目录条目（含文件大小，跳过 .git/target/node_modules 等产物目录）。条目过多会截断，请用更具体的 path。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "目录路径（默认工作区根）" },
                    "max_entries": { "type": "integer", "description": "最大条目数（默认 200）" }
                }
            }),
        }
    }

    fn as_read_only(&self) -> Option<&(dyn ReadOnlyTool + Send + Sync)> {
        Some(self)
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        list_dir_impl(ctx.workspace, ctx.policy, args).await
    }
}

/// list_dir 共享实现（串行 ToolContext 与并行只读入口共用）。
async fn list_dir_impl(workspace: &Path, policy: &Policy, args: Value) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| ".".to_string());
    let max_entries = args
        .get("max_entries")
        .and_then(Value::as_u64)
        .map(|value| (value.max(1) as usize).min(1_000))
        .unwrap_or(200);
    let abs = resolve_path_within(workspace, policy, &path)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut reader = tokio::fs::read_dir(&abs)
        .await
        .map_err(|e| format!("读取目录 {path} 失败：{e}"))?;
    while let Some(entry) = reader.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name().to_string_lossy().to_string();
        if IGNORED_DIRS.contains(&name.as_str()) {
            continue;
        }
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
        let file_type = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        let size = if file_type {
            0
        } else {
            entry.metadata().await.map(|m| m.len()).unwrap_or(0)
        };
        entries.push(json!({
            "name": name,
            "is_dir": file_type,
            "size": size,
        }));
    }
    Ok(json!({
        "path": path,
        "entries": entries,
        "truncated": truncated,
    }))
}

#[async_trait]
impl ReadOnlyTool for ListDirTool {
    async fn run_read_only(
        &self,
        workspace: &Path,
        policy: &Policy,
        args: Value,
    ) -> Result<Value, String> {
        list_dir_impl(workspace, policy, args).await
    }
}

struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search_files".into(),
            description:
                "按文件名关键字递归搜索工作区文件（只匹配文件名，不搜内容；搜内容用 grep）".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "pattern": { "type": "string" } },
                "required": ["pattern"]
            }),
        }
    }

    fn as_read_only(&self) -> Option<&(dyn ReadOnlyTool + Send + Sync)> {
        Some(self)
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        search_files_impl(ctx.workspace, args).await
    }
}

/// search_files 共享实现（只读，策略无关）。
async fn search_files_impl(workspace: &Path, args: Value) -> Result<Value, String> {
    let pattern = required_string(&args, "pattern")?.to_lowercase();
    let mut matches = Vec::new();
    collect_matches(workspace, workspace, &pattern, 0, &mut matches)
        .map_err(|e| format!("搜索失败：{e}"))?;
    Ok(json!({ "pattern": pattern, "matches": matches }))
}

#[async_trait]
impl ReadOnlyTool for SearchFilesTool {
    async fn run_read_only(
        &self,
        workspace: &Path,
        _policy: &Policy,
        args: Value,
    ) -> Result<Value, String> {
        search_files_impl(workspace, args).await
    }
}

struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "在工作区内按正则搜索文件内容，返回 文件/行号/行文本。查找代码、定义、用法优先用本工具，不要用 list_dir 盲目遍历。正则语法错误时自动按字面子串匹配。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "搜索内容：正则表达式或普通文本" },
                    "path": { "type": "string", "description": "搜索根目录，也可直接指向单个文件（默认工作区）" },
                    "glob": { "type": "string", "description": "文件名过滤，如 \"*.rs\"" },
                    "ignore_case": { "type": "boolean", "description": "忽略大小写（默认 false）" },
                    "max_results": { "type": "integer", "description": "最大匹配条数（默认 50）" }
                },
                "required": ["pattern"]
            }),
        }
    }

    fn as_read_only(&self) -> Option<&(dyn ReadOnlyTool + Send + Sync)> {
        Some(self)
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        grep_impl(ctx.workspace, ctx.policy, args).await
    }
}

/// grep 共享实现（串行 ToolContext 与并行只读入口共用）。
async fn grep_impl(workspace: &Path, policy: &Policy, args: Value) -> Result<Value, String> {
    let pattern = required_string(&args, "pattern")?;
    let root = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| ".".to_string());
    let glob = args.get("glob").and_then(Value::as_str).map(str::to_string);
    let ignore_case = args
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_results = args
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| (value.max(1) as usize).min(500))
        .unwrap_or(50);

    let abs_root = resolve_path_within(workspace, policy, &root)?;
    // 正则编译失败 → 按字面子串匹配降级（模型常传未转义的特殊字符）。
    let regex = regex::RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .build()
        .or_else(|_| {
            regex::RegexBuilder::new(&regex::escape(&pattern))
                .case_insensitive(ignore_case)
                .build()
        })
        .map_err(|e| format!("正则编译失败：{e}"))?;

    let mut state = GrepState {
        root: &abs_root,
        regex: &regex,
        glob: glob.as_deref(),
        max_results,
        matches: Vec::new(),
        truncated: false,
        files_scanned: 0,
    };
    // root 指向具体文件时直接单文件匹配：read_dir 会报「目录名称无效 (os error 267)」。
    if abs_root.is_file() {
        grep_file(&abs_root, &mut state).map_err(|e| format!("grep 搜索失败：{e}"))?;
    } else {
        grep_walk(&abs_root, &mut state, 0).map_err(|e| format!("grep 搜索失败：{e}"))?;
    }

    let mut result = json!({
        "pattern": pattern,
        "path": root,
        "files_scanned": state.files_scanned,
        "matches": state.matches,
        "truncated": state.truncated,
    });
    if state.matches.is_empty() {
        result["hint"] = json!(
            "无匹配。可尝试：ignore_case=true、放宽 pattern、更换搜索根 path、去掉 glob 限制"
        );
    }
    Ok(result)
}

#[async_trait]
impl ReadOnlyTool for GrepTool {
    async fn run_read_only(
        &self,
        workspace: &Path,
        policy: &Policy,
        args: Value,
    ) -> Result<Value, String> {
        grep_impl(workspace, policy, args).await
    }
}

/// grep 遍历状态（递归共享）。
struct GrepState<'a> {
    root: &'a Path,
    regex: &'a regex::Regex,
    glob: Option<&'a str>,
    max_results: usize,
    matches: Vec<Value>,
    truncated: bool,
    files_scanned: usize,
}

/// 递归搜索文件内容：跳过产物目录/符号链接/二进制/大文件，按行匹配。
fn grep_walk(dir: &Path, state: &mut GrepState<'_>, depth: usize) -> std::io::Result<()> {
    if depth > 12 {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir)?;
    for entry in entries.flatten() {
        if state.matches.len() >= state.max_results {
            state.truncated = true;
            return Ok(());
        }
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if file_type.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) || path.is_symlink() {
                continue;
            }
            grep_walk(&path, state, depth + 1)?;
            continue;
        }
        if let Some(glob) = state.glob {
            if !glob_matches(glob, &name) {
                continue;
            }
        }
        grep_file(&path, state)?;
    }
    Ok(())
}

/// 单个文件的按行匹配（目录遍历与「root 直接指向文件」共用）；
/// 跳过符号链接/二进制/大文件（>5MB），命中上限与截断标记由 state 维护。
fn grep_file(path: &Path, state: &mut GrepState<'_>) -> std::io::Result<()> {
    if path.is_symlink() {
        return Ok(());
    }
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > 5 * 1024 * 1024 {
        return Ok(());
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(()),
    };
    if looks_binary(&bytes) {
        return Ok(());
    }
    state.files_scanned += 1;
    let content = String::from_utf8_lossy(&bytes);
    for (index, line) in content.lines().enumerate() {
        if !state.regex.is_match(line) {
            continue;
        }
        if state.matches.len() >= state.max_results {
            state.truncated = true;
            return Ok(());
        }
        // 单文件 root 时 strip_prefix 为空串 → 回退为文件名，保证结果里能看到定位。
        let rel = match path.strip_prefix(state.root) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                relative.to_string_lossy().replace('\\', "/")
            }
            _ => path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/")),
        };
        state.matches.push(json!({
            "path": rel,
            "line_number": index + 1,
            "line": truncate_chars(line.trim(), 300),
        }));
    }
    Ok(())
}

fn collect_matches(
    root: &Path,
    dir: &Path,
    pattern: &str,
    depth: usize,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    if depth > 8 || matches.len() >= 200 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_matches(root, &entry.path(), pattern, depth + 1, matches)?;
        } else if entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .contains(pattern)
        {
            let rel = entry
                .path()
                .strip_prefix(root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| entry.path());
            matches.push(rel.to_string_lossy().replace('\\', "/"));
        }
        if matches.len() >= 200 {
            break;
        }
    }
    Ok(())
}

struct RunCommandTool;

/// run_command 的沙箱策略：工作区作用域 + Job 级隔离（允许显式降级，审计记录）+ 资源上限。
///
/// `active_process_limit` 必须放宽：命令经 `cmd /C` 执行，cmd 自身就占一个名额，
/// 沿用 `SandboxPolicy::default()` 的 1 会让之后所有子命令（hostname/python/git 等）
/// 以 1816（ERROR_NOT_ENOUGH_QUOTA）失败。口径与 `plugin.rs` / `mcp.rs` 一致。
fn run_command_sandbox_policy(workspace: &Path) -> crate::sandbox::SandboxPolicy {
    let mut policy = crate::sandbox::SandboxPolicy::for_workspace("run_command", workspace);
    policy.require_isolation = crate::sandbox::IsolationLevel::JobOnly;
    policy.allow_degraded = true;
    policy.cpu_ms = Some(60_000);
    policy.mem_mb = Some(1024);
    policy.active_process_limit = Some(32);
    policy
}

#[async_trait]
impl Tool for RunCommandTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "run_command".into(),
            description: "在工作区内执行 shell 命令（需审批）。跑测试/构建可用 timeout_secs 延长等待（默认 60 秒，最大 600 秒）。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "timeout_secs": { "type": "integer", "description": "超时秒数（默认 60，最大 600）" }
                },
                "required": ["command"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let command = required_string(&args, "command")?;
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 600))
            .unwrap_or(60);
        let cwd = args
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string)
            .map(|path| resolve_session_path(ctx, &path))
            .transpose()?
            .unwrap_or_else(|| ctx.workspace.to_path_buf());

        // 沙箱门卫：run_command 统一经 SandboxManager 执行（X01）。
        let policy = run_command_sandbox_policy(ctx.workspace);
        // 命令文本（cmd /C <command> 的命令体）同样过 deny 检查。
        if let Some(fragment) =
            crate::sandbox::SandboxCommand::deny_hit(&command, &policy.deny_programs)
        {
            return Err(format!("命令命中危险黑名单片段：{fragment}"));
        }
        let sandbox_command = crate::sandbox::SandboxCommand::new("cmd", policy.clone())
            .with_args(vec!["/C".to_string(), command.to_string()])
            .with_cwd(cwd.clone());

        let manager = crate::sandbox::default_manager();
        let process = {
            let mut manager = manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            manager
                .spawn(&sandbox_command)
                .map_err(|error| format!("沙箱拒绝执行（{}）：{error}", command))?
        };

        // 同步等待放在 blocking 线程；超时仅报错，进程仍在 Job 内受限（CPU/内存上限兜底）。
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::task::spawn_blocking(move || {
                let mut process = process;
                process.wait_output()
            }),
        )
        .await
        .map_err(|_| {
            format!("命令执行超时（{timeout_secs}s，进程仍在受限 Job 内，将被资源上限终止）")
        })?
        .map_err(|join_error| format!("命令等待失败：{join_error}"))?
        .map_err(|error| format!("沙箱执行失败：{error}"))?;

        Ok(json!({
            "command": command,
            "exit_code": output.exit_code,
            "stdout": truncate_chars(&String::from_utf8_lossy(&output.stdout), 30_000),
            "stderr": truncate_chars(&String::from_utf8_lossy(&output.stderr), 30_000),
        }))
    }
}

struct McpToolAdapter {
    full_name: String,
    server_name: String,
    tool_name: String,
    spec: ToolSpec,
    client: Arc<tokio::sync::Mutex<McpClient>>,
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let mut client = self.client.lock().await;
        client
            .call_tool(&self.tool_name, args)
            .await
            .map_err(|error| {
                format!(
                    "MCP 工具 {}:{} 失败：{error}",
                    self.server_name, self.tool_name
                )
            })
    }
}

impl std::fmt::Debug for McpToolAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpToolAdapter")
            .field("full_name", &self.full_name)
            .finish()
    }
}

/// A2-2：MCP 资源读取的泛化工具（`{server}_read_resource`）。
struct McpResourceAdapter {
    full_name: String,
    server_name: String,
    spec: ToolSpec,
    client: Arc<tokio::sync::Mutex<McpClient>>,
}

#[async_trait]
impl Tool for McpResourceAdapter {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let uri = required_string(&args, "uri")?;
        let mut client = self.client.lock().await;
        client.read_resource(&uri).await.map_err(|error| {
            format!(
                "MCP 资源读取失败（{}:{}）：{error}",
                self.server_name, uri
            )
        })
    }
}

impl std::fmt::Debug for McpResourceAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpResourceAdapter")
            .field("full_name", &self.full_name)
            .finish()
    }
}

/// A2-2：MCP 提示模板获取的泛化工具（`{server}_get_prompt`）。
struct McpPromptAdapter {
    full_name: String,
    server_name: String,
    spec: ToolSpec,
    client: Arc<tokio::sync::Mutex<McpClient>>,
}

impl std::fmt::Debug for McpPromptAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpPromptAdapter")
            .field("full_name", &self.full_name)
            .finish()
    }
}

#[async_trait]
impl Tool for McpPromptAdapter {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let name = required_string(&args, "name")?;
        let arguments = args.get("arguments").cloned();
        let mut client = self.client.lock().await;
        client.get_prompt(&name, arguments).await.map_err(|error| {
            format!(
                "MCP 模板获取失败（{}:{}）：{error}",
                self.server_name, name
            )
        })
    }
}

struct ExploreTool;

#[async_trait]
impl Tool for ExploreTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "explore".into(),
            description: "把调查任务交给只读探索子代理（只能读/搜文件），返回其调查汇报".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or("参数缺少字符串字段：query")?;
        let runner = ctx.subagent.as_ref().ok_or("子代理运行时不可用")?;
        let text = runner.run(ctx.workspace, query, true).await?;
        Ok(json!({ "mode": "explore", "text": text }))
    }
}

struct SubagentTool;

#[async_trait]
impl Tool for SubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "subagent".into(),
            description: "把独立任务委派给通用子代理（完整工具、仍需审批），返回其汇报".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "task": { "type": "string" } },
                "required": ["task"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .ok_or("参数缺少字符串字段：task")?;
        let runner = ctx.subagent.as_ref().ok_or("子代理运行时不可用")?;
        let text = runner.run(ctx.workspace, task, false).await?;
        Ok(json!({ "mode": "general", "text": text }))
    }
}

struct FanOutSubagentsTool;

/// A5-1：并行 fan-out 只读子代理（2~6 个独立调研任务同时跑）。
#[async_trait]
impl Tool for FanOutSubagentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fan_out_subagents".into(),
            description: "并行派出 2~6 个只读探索子代理，同时调研多个**相互独立**的问题（多模块分别定位、多关键词并行检索、独立子问题调研），汇总各自结论。子代理只读（不改文件、不执行命令、不联网）；任务间不能有依赖（有依赖请串行 explore/subagent）。单任务失败不影响其余，结果按输入顺序返回。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "子任务列表（每条一个独立问题/检索目标，2~6 条）"
                    },
                    "max_parallel": { "type": "integer", "description": "并发上限（默认 3，最大 4）" },
                    "timeout_secs": { "type": "integer", "description": "单个子任务超时秒数（默认 300，范围 30~900）" }
                },
                "required": ["tasks"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let tasks: Vec<String> = args
            .get("tasks")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if tasks.len() < 2 {
            return Err("tasks 至少 2 条（单个任务请直接用 explore/subagent）".to_string());
        }
        if tasks.len() > 6 {
            return Err(format!(
                "tasks 过多（{} 条 > 6）。请拆成两批分别 fan-out",
                tasks.len()
            ));
        }
        let max_parallel = args
            .get("max_parallel")
            .and_then(Value::as_u64)
            .unwrap_or(3)
            .clamp(1, 4) as usize;
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(300)
            .clamp(30, 900);
        let fanout = ctx
            .fanout
            .clone()
            .ok_or("当前环境不支持并行子代理（子代理内/CLI/评测环境不可用）")?;
        let abort = ctx.abort;
        // 取消桥：主回合 abort（用户急停/流断开）→ far-out 取消标志 → 子代理 abort。
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let config = crate::fleet::FanOutConfig {
            max_parallel,
            budget: crate::fleet::Budget {
                max_duration_secs: timeout_secs.saturating_mul(2).max(120),
                ..Default::default()
            },
            per_worker_timeout: Some(std::time::Duration::from_secs(timeout_secs)),
            cancelled: Some(std::sync::Arc::clone(&cancelled)),
            ..Default::default()
        };
        let future = crate::subagent::fan_out_subagents(
            fanout.provider,
            fanout.workspace,
            fanout.model,
            fanout.depth,
            fanout.max_turns,
            tasks.clone(),
            config,
        );
        tokio::pin!(future);
        let report = loop {
            tokio::select! {
                result = &mut future => break result?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                    if let Some(flag) = abort {
                        if flag.load(std::sync::atomic::Ordering::SeqCst) {
                            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                }
            }
        };
        let succeeded = report.succeeded().len();
        let failed = report.failed().len();
        let results: Vec<Value> = report
            .outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                json!({
                    "index": index + 1,
                    "task": tasks.get(index).cloned().unwrap_or_default(),
                    "ok": outcome.ok,
                    "status": outcome.status,
                    "output": outcome.output,
                    "error": outcome.error,
                })
            })
            .collect();
        Ok(json!({
            "succeeded": succeeded,
            "failed": failed,
            "results": results,
            "hint": if failed == 0 {
                "全部子任务成功".to_string()
            } else {
                format!("{failed} 个子任务未成功：Failed/TimedOut 可单独重试，Cancelled/Aborted 为整体取消/预算中止")
            },
        }))
    }
}

struct UseSkillTool;

#[async_trait]
impl Tool for UseSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "use_skill".into(),
            description: "读取已加载技能（SKILL.md）的完整指令并按其流程执行；名称可通过 /skills 或技能清单查看".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "task": { "type": "string" }
                },
                "required": ["name"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("参数缺少字符串字段：name")?;
        let Some(skill) = ctx.skills.get_enabled(name) else {
            let available = ctx
                .skills
                .list_enabled()
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "未找到技能或技能已禁用：{name}；可用技能：{available}"
            ));
        };
        let task = args.get("task").and_then(Value::as_str).unwrap_or_default();
        Ok(json!({
            "skill": skill.name,
            "description": skill.description,
            "task": task,
            "instructions": skill.instructions,
        }))
    }
}

/// 向用户提问并等待回答（信息不足/需求含糊/关键分歧时使用）。
/// 回合会挂起直到用户答复或超时；无 UI 通道时明确报错，让模型改为书面提问。
struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ask_user".into(),
            description: "信息不足、需求含糊或存在会显著影响结果的关键分歧时，向用户提问并等待回答（回合暂停直到用户答复）。问题要具体、一次只问最关键的一两个点；能用选项固定答案时给出 options。已经明确的常规操作不要用它确认。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "要向用户提出的问题（简洁明确，一次只问一件事）" },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选：2-4 个备选答案，用户可直接点选"
                    }
                },
                "required": ["question"]
            }),
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("参数缺少字符串字段：question")?
            .to_string();
        let options: Vec<String> = args
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .take(4)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let Some(questioner) = ctx.questioner else {
            return Err("当前运行环境没有可用的用户问答通道（无 UI 连接）。请把你的问题直接写进最终回复向用户提出，并给出你建议的默认方案。".to_string());
        };
        let request = crate::question::UserQuestion {
            question_id: uuid::Uuid::new_v4().to_string(),
            question,
            options,
        };
        match questioner.ask(&request).await {
            Some(answer) if !answer.answer.trim().is_empty() => Ok(json!({
                "answered": true,
                "answer": answer.answer,
            })),
            // 超时/中止/空回答：不给模型「卡住」的机会——明确告知并允许继续。
            _ => Ok(json!({
                "answered": false,
                "note": "用户未在时限内回答。请基于已有信息继续执行，并在最终回复里把不确定的部分标注出来。",
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_path_within_accepts_absolute_new_file_and_rejects_outside() {
        let temp = std::env::temp_dir().join("owo-resolve-path-check");
        std::fs::create_dir_all(&temp).expect("创建临时目录");
        let policy = Policy::new(&temp);
        // 绝对路径 + 文件尚不存在（曾实测被误判「路径越界」的场景）。
        let absolute = temp.join("hello.txt");
        assert!(resolve_path_within(&temp, &policy, absolute.to_str().unwrap()).is_ok());
        // 相对路径（含尚不存在的子目录）。
        assert!(resolve_path_within(&temp, &policy, "nested/hello.txt").is_ok());
        // 工作区外（父目录）必须仍然拒绝。
        let outside = temp.parent().unwrap().join("owo-resolve-path-outside.txt");
        assert!(resolve_path_within(&temp, &policy, outside.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[cfg(windows)]
    #[test]
    fn run_command_policy_allows_child_processes() {
        // 回归：Job 的 active_process_limit 若沿用默认 1，`cmd /C <子命令>` 会以
        // 1816（ERROR_NOT_ENOUGH_QUOTA）失败——run_command 必须能起子进程。
        let temp = std::env::temp_dir().join("owo-run-command-child-probe");
        std::fs::create_dir_all(&temp).expect("创建临时目录");
        let command = crate::sandbox::SandboxCommand::new("cmd", run_command_sandbox_policy(&temp))
            .with_args(vec!["/C".to_string(), "hostname".to_string()])
            .with_cwd(temp.clone());
        let manager = crate::sandbox::default_manager();
        let mut process = {
            let mut manager = manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            manager.spawn(&command).expect("沙箱应允许执行")
        };
        let output = process.wait_output().expect("等待输出");
        assert_eq!(
            output.exit_code,
            0,
            "子进程应能启动；stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn sanitizes_tool_names_for_model_api() {
        assert_eq!(
            sanitize_tool_name("owo.plugin.example-hello"),
            "owo_plugin_example-hello"
        );
        assert_eq!(sanitize_tool_name("echo"), "echo");
        assert_eq!(sanitize_tool_name("a b/c"), "a_b_c");
    }

    #[test]
    fn mcp_tool_prefix_sanitizes_plugin_id() {
        assert_eq!(
            mcp_tool_prefix("owo.plugin.translate"),
            "owo_plugin_translate_"
        );
        assert_eq!(
            mcp_tool_prefix("owo-plugin-clipboard"),
            "owo-plugin-clipboard_"
        );
    }

    struct NamedTool {
        name: String,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: String::new(),
                input_schema: serde_json::json!({}),
            }
        }

        async fn run(
            &self,
            _ctx: &mut ToolContext<'_>,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::Value::Null)
        }
    }

    #[test]
    fn remove_prefix_unregisters_only_matching_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(NamedTool {
            name: "owo_plugin_demo_translate".to_string(),
        });
        registry.register(NamedTool {
            name: "owo_plugin_demo_clipboard".to_string(),
        });
        registry.register(NamedTool {
            name: "builtin_tool".to_string(),
        });
        let removed = registry.remove_prefix("owo_plugin_demo_");
        assert_eq!(removed, 2);
        let names: Vec<String> = registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        assert!(!names
            .iter()
            .any(|name| name.starts_with("owo_plugin_demo_")));
        assert!(names.iter().any(|name| name == "builtin_tool"));
    }

    // ---------- 效能核心改造（B1/B2）测试 ----------

    /// 构造一次性临时工作区（用后删除；失败不阻断断言）。
    fn temp_workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("owo-tools-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("创建临时工作区失败");
        dir
    }

    type TestEnv = (
        PathBuf,
        Session,
        Policy,
        Arc<Mutex<AuditLog>>,
        SkillRegistry,
        Arc<Mutex<crate::ElementRegistry>>,
    );

    fn test_env(tag: &str) -> TestEnv {
        let workspace = temp_workspace(tag);
        let session = Session::new(&workspace, "mock", None);
        let policy = Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(AuditLog::default()));
        let skills = SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        (workspace, session, policy, audit, skills, elements)
    }

    fn make_ctx<'a>(env: &'a mut TestEnv) -> ToolContext<'a> {
        let (workspace, session, policy, audit, skills, elements) = env;
        ToolContext {
            workspace,
            policy,
            session,
            audit,
            subagent: None,
            skills,
            elements,
            questioner: None,
            fanout: None,
            abort: None,
        }
    }

    fn cleanup(workspace: &Path) {
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn ask_user_without_channel_reports_error() {
        // 无 UI 通道（CLI/子代理/测试）时明确报错，让模型改为书面提问，而不是挂死。
        let mut env = test_env("ask-user-none");
        let mut ctx = make_ctx(&mut env);
        let error = AskUserTool
            .run(&mut ctx, json!({ "question": "先做 A 还是 B？" }))
            .await
            .unwrap_err();
        assert!(error.contains("没有可用的用户问答通道"), "{error}");
        cleanup(&env.0);
    }

    /// 回显提问通道：直接返回答案，模拟用户在提问卡上作答。
    struct EchoQuestioner;

    #[async_trait]
    impl crate::question::Questioner for EchoQuestioner {
        async fn ask(
            &self,
            question: &crate::question::UserQuestion,
        ) -> Option<crate::question::QuestionAnswer> {
            Some(crate::question::QuestionAnswer {
                question_id: question.question_id.clone(),
                answer: format!("已收到：{}", question.question),
            })
        }
    }

    #[tokio::test]
    async fn ask_user_returns_answer_through_channel() {
        // 有通道时提问经 Questioner 拿回用户答案，以 answered=true 回给模型。
        let mut env = test_env("ask-user-ok");
        let questioner = EchoQuestioner;
        let result = {
            let mut ctx = make_ctx(&mut env);
            ctx.questioner = Some(&questioner);
            AskUserTool
                .run(
                    &mut ctx,
                    json!({ "question": "先做 A 还是 B？", "options": ["A", "B"] }),
                )
                .await
                .unwrap()
        };
        assert_eq!(result["answered"], true);
        assert_eq!(result["answer"], "已收到：先做 A 还是 B？");
        cleanup(&env.0);
    }

    #[tokio::test]
    async fn grep_finds_content_and_ignores_build_dirs() {
        let mut env = test_env("grep");
        let workspace = env.0.clone();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::create_dir_all(workspace.join("target")).unwrap();
        std::fs::write(
            workspace.join("src/a.rs"),
            "fn main() {\n    // hello world\n}\n",
        )
        .unwrap();
        std::fs::write(workspace.join("target/junk.rs"), "hello\n").unwrap();
        std::fs::write(workspace.join("notes.txt"), "nothing here\n").unwrap();

        let mut ctx = make_ctx(&mut env);
        let result = GrepTool
            .run(&mut ctx, json!({ "pattern": "hello" }))
            .await
            .unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "target/ 应被忽略：{result}");
        assert_eq!(matches[0]["path"], "src/a.rs");
        assert_eq!(matches[0]["line_number"], 2);
        assert!(matches[0]["line"].as_str().unwrap().contains("hello world"));
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn grep_falls_back_to_literal_on_invalid_regex() {
        let mut env = test_env("grep-lit");
        let workspace = env.0.clone();
        std::fs::write(workspace.join("a.txt"), "price: (unclosed\n").unwrap();

        let mut ctx = make_ctx(&mut env);
        let result = GrepTool
            .run(&mut ctx, json!({ "pattern": "(unclosed" }))
            .await
            .unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "非法正则应降级字面匹配：{result}");
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn grep_glob_filters_and_gives_hint_on_empty() {
        let mut env = test_env("grep-glob");
        let workspace = env.0.clone();
        std::fs::write(workspace.join("a.rs"), "needle\n").unwrap();
        std::fs::write(workspace.join("b.txt"), "needle\n").unwrap();

        let mut ctx = make_ctx(&mut env);
        let result = GrepTool
            .run(&mut ctx, json!({ "pattern": "needle", "glob": "*.rs" }))
            .await
            .unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "a.rs");

        let result = GrepTool
            .run(&mut ctx, json!({ "pattern": "absent" }))
            .await
            .unwrap();
        assert!(result["matches"].as_array().unwrap().is_empty());
        assert!(result["hint"].as_str().unwrap().contains("无匹配"));
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn grep_accepts_file_as_root() {
        let mut env = test_env("grep-file-root");
        let workspace = env.0.clone();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        let target = workspace.join("src/a.rs");
        std::fs::write(&target, "fn main() {\n    // hello world\n}\n").unwrap();
        std::fs::write(workspace.join("src/b.rs"), "hello hello\n").unwrap();

        let mut ctx = make_ctx(&mut env);
        // 相对路径指向文件：不再报「目录名称无效 (os error 267)」，只搜该文件。
        let result = GrepTool
            .run(&mut ctx, json!({ "pattern": "hello", "path": "src/a.rs" }))
            .await
            .unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "root 指向文件时应只搜该文件：{result}");
        assert_eq!(matches[0]["path"], "a.rs");
        assert_eq!(matches[0]["line_number"], 2);
        assert_eq!(result["files_scanned"], 1);

        // 绝对路径指向文件同样可用。
        let result = GrepTool
            .run(
                &mut ctx,
                json!({ "pattern": "hello", "path": target.to_string_lossy().replace('\\', "/") }),
            )
            .await
            .unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn read_file_renders_line_numbers_and_paginates() {
        let mut env = test_env("read");
        let workspace = env.0.clone();
        let body: String = (1..=10).map(|n| format!("line-{n}\n")).collect();
        std::fs::write(workspace.join("ten.txt"), body).unwrap();

        let mut ctx = make_ctx(&mut env);
        let result = ReadFileTool
            .run(
                &mut ctx,
                json!({ "path": "ten.txt", "offset": 3, "limit": 2 }),
            )
            .await
            .unwrap();
        assert_eq!(result["total_lines"], 10);
        assert_eq!(result["start_line"], 3);
        assert_eq!(result["end_line"], 4);
        assert_eq!(result["truncated"], true);
        let content = result["content"].as_str().unwrap();
        assert!(content.contains("     3\tline-3"), "行号格式：{content}");
        assert!(content.contains("     4\tline-4"));
        assert!(!content.contains("line-5\n"));
        assert!(content.contains("offset=5"), "截断续读提示：{content}");

        // 全量读取（limit 覆盖全部行）不应截断。
        let result = ReadFileTool
            .run(&mut ctx, json!({ "path": "ten.txt" }))
            .await
            .unwrap();
        assert_eq!(result["truncated"], false);
        assert_eq!(result["end_line"], 10);
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn read_file_rejects_binary_and_missing() {
        let mut env = test_env("read-bin");
        let workspace = env.0.clone();
        std::fs::write(workspace.join("blob.bin"), b"abc\x00def").unwrap();

        let mut ctx = make_ctx(&mut env);
        let error = ReadFileTool
            .run(&mut ctx, json!({ "path": "blob.bin" }))
            .await
            .unwrap_err();
        assert!(error.contains("二进制"), "{error}");
        let error = ReadFileTool
            .run(&mut ctx, json!({ "path": "missing.txt" }))
            .await
            .unwrap_err();
        assert!(error.contains("失败"), "{error}");
        cleanup(&workspace);
    }

    #[test]
    fn main_registry_contains_all_capability_tools() {
        // 回归保护：批次 5/6 新增能力工具必须在主注册表（注册表集中式，
        // 漏注册 = 模型看不到工具，功能静默失效）。
        let names: Vec<String> = ToolRegistry::new()
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        for expected in [
            "read_file",
            "write_file",
            "edit_file",
            "multi_edit",
            "list_dir",
            "search_files",
            "grep",
            "run_command",
            "explore",
            "subagent",
            "fan_out_subagents",
            "use_skill",
            "ask_user",
            "update_plan",
            "web_search",
            "web_fetch",
            "git_status",
            "git_diff",
            "git_log",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "主注册表缺少工具：{expected}（实际：{names:?}）"
            );
        }
        // headless（评测）也带代码工程与 git 能力，但不含桌面/浏览器副作用工具，
        // 也不含委派类（subagent/fan_out_subagents）——委派会另建整表，间接可达副作用工具。
        let headless: Vec<String> = ToolRegistry::headless()
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        for expected in ["multi_edit", "web_fetch", "web_search", "git_status", "git_diff", "git_log"] {
            assert!(
                headless.iter().any(|name| name == expected),
                "headless 注册表缺少工具：{expected}"
            );
        }
        for excluded in ["subagent", "fan_out_subagents"] {
            assert!(
                !headless.iter().any(|name| name == excluded),
                "headless 注册表不应含委派工具：{excluded}"
            );
        }
        assert!(!headless.iter().any(|name| name.starts_with("browser_")));
        assert!(!headless.iter().any(|name| name.starts_with("desktop_")));
    }

    #[tokio::test]
    async fn multi_edit_applies_all_edits_in_order() {
        let mut env = test_env("multi-edit");
        let workspace = env.0.clone();
        let original = "fn main() {\n    let version = \"1.0\";\n    println!(\"v1.0\");\n}\n";
        std::fs::write(workspace.join("app.rs"), original).unwrap();

        let mut ctx = make_ctx(&mut env);
        // 顺序应用：第 2 处的 old_str 匹配第 1 处替换后的内容。
        let result = MultiEditTool
            .run(
                &mut ctx,
                json!({
                    "path": "app.rs",
                    "edits": [
                        { "old_str": "let version = \"1.0\";", "new_str": "let version = \"2.0\";" },
                        { "old_str": "println!(\"v1.0\");", "new_str": "println!(\"v{version}\");" }
                    ]
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["applied"], 2, "{result}");
        let updated = std::fs::read_to_string(workspace.join("app.rs")).unwrap();
        assert!(updated.contains("let version = \"2.0\";"), "{updated}");
        assert!(updated.contains("println!(\"v{version}\");"), "{updated}");
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn multi_edit_is_atomic_on_failure() {
        let mut env = test_env("multi-edit-atomic");
        let workspace = env.0.clone();
        let original = "alpha\nbeta\ngamma\n";
        std::fs::write(workspace.join("data.txt"), original).unwrap();

        let mut ctx = make_ctx(&mut env);
        // 第 2 处失败（old_str 不存在）：整批不落盘。
        let error = MultiEditTool
            .run(
                &mut ctx,
                json!({
                    "path": "data.txt",
                    "edits": [
                        { "old_str": "alpha", "new_str": "ALPHA" },
                        { "old_str": "不存在的片段", "new_str": "x" }
                    ]
                }),
            )
            .await
            .unwrap_err();
        assert!(error.contains("2/2") && error.contains("整批未应用"), "{error}");
        assert_eq!(
            std::fs::read_to_string(workspace.join("data.txt")).unwrap(),
            original,
            "失败时原文件必须保持不变"
        );

        // 多处歧义同样整体失败。
        std::fs::write(workspace.join("data.txt"), "same\nsame\n").unwrap();
        let error = MultiEditTool
            .run(
                &mut ctx,
                json!({
                    "path": "data.txt",
                    "edits": [
                        { "old_str": "same", "new_str": "x" },
                        { "old_str": "gamma", "new_str": "y" }
                    ]
                }),
            )
            .await
            .unwrap_err();
        assert!(error.contains("出现 2 次"), "{error}");
        assert_eq!(
            std::fs::read_to_string(workspace.join("data.txt")).unwrap(),
            "same\nsame\n"
        );
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_and_reports_ambiguity() {
        let mut env = test_env("edit");
        let workspace = env.0.clone();
        std::fs::write(
            workspace.join("code.rs"),
            "fn a() {}\nfn b() {}\nfn c() {}\n",
        )
        .unwrap();

        let mut ctx = make_ctx(&mut env);
        let result = EditFileTool
            .run(
                &mut ctx,
                json!({ "path": "code.rs", "old_str": "fn b() {}", "new_str": "fn b(x: u32) {}" }),
            )
            .await
            .unwrap();
        assert_eq!(result["replaced"], 1);
        let updated = std::fs::read_to_string(workspace.join("code.rs")).unwrap();
        assert!(updated.contains("fn b(x: u32) {}"));
        assert!(updated.contains("fn a() {}"), "未匹配部分不动");

        // 未找到。
        let error = EditFileTool
            .run(
                &mut ctx,
                json!({ "path": "code.rs", "old_str": "absent", "new_str": "x" }),
            )
            .await
            .unwrap_err();
        assert!(error.contains("未找到 old_str"), "{error}");

        // 多处命中且未设 replace_all。
        std::fs::write(workspace.join("dup.rs"), "same\nsame\n").unwrap();
        let error = EditFileTool
            .run(
                &mut ctx,
                json!({ "path": "dup.rs", "old_str": "same", "new_str": "diff" }),
            )
            .await
            .unwrap_err();
        assert!(error.contains("出现 2 次"), "{error}");

        // replace_all。
        let result = EditFileTool
            .run(
                &mut ctx,
                json!({ "path": "dup.rs", "old_str": "same", "new_str": "diff", "replace_all": true }),
            )
            .await
            .unwrap();
        assert_eq!(result["replaced"], 2);
        assert_eq!(
            std::fs::read_to_string(workspace.join("dup.rs")).unwrap(),
            "diff\ndiff\n"
        );
        cleanup(&workspace);
    }

    #[tokio::test]
    async fn edit_file_snapshot_enables_revert() {
        let mut env = test_env("edit-snap");
        let workspace = env.0.clone();
        std::fs::write(workspace.join("orig.txt"), "before\n").unwrap();

        let mut ctx = make_ctx(&mut env);
        EditFileTool
            .run(
                &mut ctx,
                json!({ "path": "orig.txt", "old_str": "before", "new_str": "after" }),
            )
            .await
            .unwrap();
        let snapshots = env.1.snapshots;
        assert_eq!(snapshots.len(), 1, "编辑后应有快照（diff/revert 依赖）");
        let entry = snapshots.values().next().unwrap();
        let decoded = BASE64
            .decode(entry.original_b64.as_deref().unwrap_or_default())
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&decoded), "before\n");
        cleanup(&workspace);
    }

    #[test]
    fn glob_and_truncate_helpers() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(glob_matches("*.RS", "main.rs"));
        assert!(!glob_matches("*.rs", "main.txt"));
        assert!(glob_matches("Cargo.toml", "cargo.TOML"));
        assert!(!glob_matches("Cargo.toml", "Cargo.lock"));
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(truncate_chars("abc", 3), "abc");
        assert_eq!(truncate_chars("中文安全截断", 2), "中文…");
        assert!(looks_binary(b"abc\x00def"));
        assert!(!looks_binary(b"abc\ndef"));
    }
}
