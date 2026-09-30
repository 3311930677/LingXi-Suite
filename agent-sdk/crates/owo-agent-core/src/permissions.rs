use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Read,
    Write,
    Execute,
    Inject,
}

impl Level {
    pub fn label(&self) -> &'static str {
        match self {
            Level::Read => "read",
            Level::Write => "write",
            Level::Execute => "execute",
            Level::Inject => "inject",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

/// 规则表决策（可持久化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleDecision {
    Allow,
    Deny,
}

/// 用户记住的权限规则（审批卡片「记住」产生；持久化到 `settings.json` 的 `permissions.rules`）。
///
/// 匹配语义（[`Policy::rule_matches`]）：
/// - `pattern == "*"`：该工具任意参数（desktop_* / browser_* 等无结构参数的工具）；
/// - `pattern == "cargo *"`：命令首 token 前缀匹配（run_command）；
/// - `pattern == "src/**"`：路径父目录前缀匹配（read/write/edit_file）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// 工具名（精确匹配）。
    pub tool: String,
    /// 归一化匹配模式。
    pub pattern: String,
    pub decision: RuleDecision,
    /// 失效时间（Unix 毫秒）；`None` = 永久。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    /// 备注（审计可读）。
    #[serde(default)]
    pub note: String,
}

/// 危险命令硬拒绝片段：这些命令**永不进入规则表**（remember 无效、永远 Ask/Deny）。
pub const HARD_DENY_FRAGMENTS: &[&str] = &[
    "rm -rf",
    "sudo",
    "shutdown",
    "format c:",
    "rd /s",
    "remove-item -recurse",
    "del /s",
    "git push",
    "git reset --hard",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool: String,
    pub args: Value,
    pub level: Level,
    pub reason: String,
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn decide(&self, request: &PermissionRequest) -> Decision;
}

/// 测试/自动化用：统一放行或拒绝。
pub struct AutoApprover {
    pub allow: bool,
}

#[async_trait]
impl Approver for AutoApprover {
    async fn decide(&self, _request: &PermissionRequest) -> Decision {
        if self.allow {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}

/// 权限策略：硬拒绝（越界/危险命令） > 只读限制 > 用户规则表 > 默认（读放行、写/执行询问）。
/// 作用域：所有文件/命令路径必须位于 workspace 内。
pub struct Policy {
    workspace: PathBuf,
    deny_command_fragments: Vec<String>,
    /// 运行时追加的危险命令片段（热生效，与基础列表合并判断）。
    runtime_deny: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    read_only: Arc<AtomicBool>,
    /// 用户记住的权限规则（remember 产生；跨会话热共享，启动时由 settings 灌入）。
    rules: std::sync::Arc<std::sync::RwLock<Vec<PermissionRule>>>,
    /// 会话级临时规则（A6-1「本会话内允许」）：仅内存，不落盘、进程重启失效。
    session_rules: std::sync::Arc<std::sync::Mutex<Vec<PermissionRule>>>,
}

impl Policy {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            deny_command_fragments: HARD_DENY_FRAGMENTS
                .iter()
                .map(|fragment| (*fragment).to_string())
                .collect(),
            runtime_deny: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            read_only: Arc::new(AtomicBool::new(false)),
            rules: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            session_rules: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// 派生一个路径作用域为指定工作区的策略：继承危险命令片段、运行时 deny、只读开关与规则表（共享热状态）。
    ///
    /// 服务端以固定启动工作区构建 Policy，而会话工作区由用户任选；回合开始时用会话工作区派生，
    /// 否则所有文件工具都会因「路径位于工作区之外」被拒。
    pub fn scoped_to(&self, workspace: impl Into<PathBuf>) -> Policy {
        Policy {
            workspace: workspace.into(),
            deny_command_fragments: self.deny_command_fragments.clone(),
            runtime_deny: Arc::clone(&self.runtime_deny),
            read_only: Arc::clone(&self.read_only),
            rules: Arc::clone(&self.rules),
            session_rules: Arc::clone(&self.session_rules),
        }
    }

    /// 只读策略（Plan 模式）：写/执行/注入一律拒绝。
    pub fn read_only(workspace: impl Into<PathBuf>) -> Self {
        let policy = Self::new(workspace);
        policy.read_only.store(true, Ordering::Relaxed);
        policy
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only.store(read_only, Ordering::Relaxed);
    }

    pub(crate) fn set_read_only_runtime(&self, read_only: bool) {
        self.read_only.store(read_only, Ordering::Relaxed);
    }

    /// 追加额外危险命令片段（deny 优先；写入基础列表，构造时静态）。
    pub fn add_deny_command(&mut self, fragment: impl Into<String>) {
        let fragment = fragment.into().to_lowercase();
        if !self.deny_command_fragments.contains(&fragment) {
            self.deny_command_fragments.push(fragment);
        }
    }

    /// 运行时追加危险命令片段（热生效，不重建 Policy；进程重启后由 settings 恢复）。
    pub fn add_runtime_deny(&self, fragment: impl Into<String>) {
        let fragment = fragment.into().to_lowercase();
        if let Ok(mut runtime) = self.runtime_deny.lock() {
            if !runtime.contains(&fragment) {
                runtime.push(fragment);
            }
        }
    }

    pub(crate) fn replace_runtime_deny(&self, fragments: &[String]) {
        if let Ok(mut runtime) = self.runtime_deny.lock() {
            runtime.clear();
            for fragment in fragments {
                let fragment = fragment.to_lowercase();
                if !fragment.is_empty() && !runtime.contains(&fragment) {
                    runtime.push(fragment);
                }
            }
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only.load(Ordering::Relaxed)
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// 全部危险命令片段（基础 + 运行时，去重；诊断/可视化用只读访问器）。
    pub fn deny_fragments(&self) -> Vec<String> {
        let runtime = self
            .runtime_deny
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let mut all = self.deny_command_fragments.clone();
        for fragment in runtime {
            if !all.contains(&fragment) {
                all.push(fragment);
            }
        }
        all
    }

    /// 工具 → 权限级别矩阵（内置映射，诊断面板展示）。
    pub fn tool_levels() -> Vec<(String, Level)> {
        [
            ("read_file", Level::Read),
            ("list_dir", Level::Read),
            ("search_files", Level::Read),
            ("grep", Level::Read),
            ("screen_ocr", Level::Read),
            ("desktop_window_ocr", Level::Read),
            ("ocr_region", Level::Read),
            ("desktop_foreground", Level::Read),
            ("desktop_window_list", Level::Read),
            ("desktop_wait", Level::Read),
            ("desktop_wait_until", Level::Read),
            ("browser_snapshot", Level::Read),
            ("screen_vision", Level::Read),
            ("vision_verify", Level::Read),
            ("vision_ground", Level::Read),
            ("write_file", Level::Write),
            ("edit_file", Level::Write),
            ("browser_screenshot", Level::Write),
            ("browser_download_image", Level::Write),
            ("ask_user", Level::Read),
            ("update_plan", Level::Read),
            ("run_command", Level::Execute),
            ("browser_navigate", Level::Execute),
            ("browser_search", Level::Execute),
            ("browser_click", Level::Execute),
            ("browser_type", Level::Execute),
            ("browser_press", Level::Execute),
            ("browser_close", Level::Execute),
            ("desktop_click", Level::Inject),
            ("desktop_type", Level::Inject),
            ("desktop_key", Level::Inject),
            ("desktop_shortcut", Level::Inject),
            ("desktop_activate", Level::Inject),
            ("desktop_launch", Level::Inject),
            ("desktop_scroll", Level::Inject),
        ]
        .iter()
        .map(|(tool, level)| (tool.to_string(), *level))
        .collect()
    }

    pub fn level_for(tool: &str) -> Level {
        match tool {
            // ask_user 只与用户交互、不触碰工作区，免审批（否则会出现「要审批才能提问」的悖论）；
            // update_plan 只写会话内存态的计划，同样免审批；
            // git_* 为固定子命令白名单的只读包装（A3-4），无写副作用。
            "read_file" | "list_dir" | "search_files" | "grep" | "ask_user" | "update_plan"
            | "git_status" | "git_diff" | "git_log" => Level::Read,
            "write_file" | "edit_file" | "multi_edit" => Level::Write,
            "run_command" => Level::Execute,
            // web 出网请求：虽是「读」，但会向外部发送用户上下文相关的查询，
            // 且存在 SSRF 面（已有内网防护），保持 Execute 档留一道审批闸。
            "web_search" | "web_fetch" => Level::Execute,
            "screen_ocr"
            | "desktop_window_ocr"
            | "ocr_region"
            | "desktop_foreground"
            | "desktop_window_list"
            | "desktop_wait"
            | "desktop_wait_until"
            | "browser_snapshot"
            | "screen_vision"
            | "vision_verify"
            | "vision_ground" => Level::Read,
            "desktop_click" | "desktop_type" | "desktop_key" | "desktop_shortcut"
            | "desktop_activate" | "desktop_launch" | "desktop_scroll" => Level::Inject,
            "browser_navigate" | "browser_search" | "browser_click" | "browser_type"
            | "browser_press" | "browser_close" => Level::Execute,
            "browser_screenshot" | "browser_download_image" => Level::Write,
            _ => Level::Execute,
        }
    }

    /// 解析并校验路径位于 workspace 内（文件可尚不存在，校验父级）。
    pub fn resolve_within_workspace(&self, path: &str) -> Result<PathBuf, String> {
        resolve_within(&self.workspace, path)
    }

    pub fn evaluate(&self, tool: &str, args: &Value) -> PermissionRequest {
        let request_id = uuid::Uuid::new_v4().to_string();
        let level = Self::level_for(tool);
        let reason = match tool {
            "read_file" | "write_file" | "edit_file" => {
                let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
                match self.resolve_within_workspace(path) {
                    Ok(_) => format!("{level} 文件操作（工作区内）", level = level.label()),
                    Err(e) => format!("拒绝：{e}"),
                }
            }
            "list_dir" | "search_files" | "grep" => "目录/搜索操作".to_string(),
            "run_command" => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let lower = command.to_lowercase();
                let denied = self
                    .deny_fragments()
                    .iter()
                    .any(|frag| lower.contains(frag));
                if denied {
                    "拒绝：命令命中危险模式".to_string()
                } else {
                    format!("执行命令：{command}")
                }
            }
            "ask_user" => "向用户提问并等待回答".to_string(),
            _ => format!("工具 {tool} 需要审批"),
        };
        PermissionRequest {
            request_id,
            tool: tool.to_string(),
            args: args.clone(),
            level,
            reason,
        }
    }

    /// 工具执行前的最终判定（拒绝原因通过 request.reason 表达）。
    ///
    /// 优先级：越界/危险命令硬拒绝 → 只读限制 → 用户规则表 → 默认（读放行，写/执行/注入询问）。
    pub fn decision(&self, request: &PermissionRequest) -> Decision {
        if request.reason.starts_with("拒绝") {
            return Decision::Deny;
        }
        if self.is_read_only() && request.level != Level::Read {
            return Decision::Deny;
        }
        match request.level {
            Level::Read => Decision::Allow,
            Level::Write | Level::Execute | Level::Inject => {
                // 用户记住的规则（remember）：命中即生效，不再打扰。
                if let Some(rule) = self.matching_rule(&request.tool, &request.args) {
                    return match rule.decision {
                        RuleDecision::Allow => Decision::Allow,
                        RuleDecision::Deny => Decision::Deny,
                    };
                }
                Decision::Ask
            }
        }
    }

    // ────────────────────────── 规则表（remember） ──────────────────────────

    /// 工具+参数是否可被「记住」：危险命令硬拒绝清单**永不可入表**。
    pub fn rememberable(tool: &str, args: &Value) -> bool {
        if tool == "run_command" {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            return !HARD_DENY_FRAGMENTS
                .iter()
                .any(|fragment| command.contains(fragment));
        }
        true
    }

    /// `(tool, args)` → 归一化规则模式（见 [`PermissionRule`] 文档）。
    pub fn rule_pattern(workspace: &Path, tool: &str, args: &Value) -> String {
        match tool {
            "run_command" => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                match command.split_whitespace().next() {
                    Some(token) => format!("{token} *"),
                    None => "*".to_string(),
                }
            }
            "read_file" | "write_file" | "edit_file" => {
                let raw = args.get("path").and_then(Value::as_str).unwrap_or_default();
                let normalized = normalize_rule_path(workspace, raw);
                match normalized.rsplit_once('/') {
                    Some((parent, _)) if !parent.is_empty() => format!("{parent}/**"),
                    _ => "*".to_string(),
                }
            }
            _ => "*".to_string(),
        }
    }

    /// 记住一条允许规则（审批卡片「记住」）：同 `(tool, pattern)` 覆盖旧值。
    ///
    /// 不可记住的工具（危险命令）返回 `None`，调用方据此提示「该操作不可记住」。
    pub fn remember_rule(
        &self,
        tool: &str,
        args: &Value,
        ttl: Option<std::time::Duration>,
    ) -> Option<PermissionRule> {
        if !Self::rememberable(tool, args) {
            return None;
        }
        let rule = PermissionRule {
            tool: tool.to_string(),
            pattern: Self::rule_pattern(&self.workspace, tool, args),
            decision: RuleDecision::Allow,
            expires_at_ms: ttl.map(|ttl| now_ms().saturating_add(ttl.as_millis() as u64)),
            note: format!("审批卡片记住（{}）", tool),
        };
        if let Ok(mut rules) = self.rules.write() {
            rules.retain(|existing| {
                !(existing.tool == rule.tool && existing.pattern == rule.pattern)
            });
            rules.push(rule.clone());
        }
        Some(rule)
    }

    /// 覆盖规则表（启动时由 `settings.permissions.rules` 灌入）。
    pub fn replace_rules(&self, rules: Vec<PermissionRule>) {
        if let Ok(mut guard) = self.rules.write() {
            *guard = rules;
        }
    }

    /// 导出规则表（持久化用）。
    pub fn rules(&self) -> Vec<PermissionRule> {
        self.rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 记住一条**会话级**允许规则（A6-1「本会话内允许」）：仅内存，不落盘，
    /// 进程重启即失效；同 `(tool, pattern)` 覆盖旧值。不可记住的工具返回 `None`。
    pub fn remember_session_rule(&self, tool: &str, args: &Value) -> Option<PermissionRule> {
        let rule = self.remember_rule(tool, args, None)?;
        if let Ok(mut session) = self.session_rules.lock() {
            session.retain(|existing| !(existing.tool == rule.tool && existing.pattern == rule.pattern));
            session.push(rule.clone());
        }
        // 会话级规则不进持久表：从 rules 里移除 remember_rule 刚写入的同键项。
        if let Ok(mut rules) = self.rules.write() {
            rules.retain(|existing| !(existing.tool == rule.tool && existing.pattern == rule.pattern));
        }
        Some(rule)
    }

    /// 导出会话级临时规则（诊断/规则面板展示）。
    pub fn session_rules(&self) -> Vec<PermissionRule> {
        self.session_rules
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 删除一条规则（B4-1 规则管理）：`session = true` 删内存临时表，否则删持久表。
    /// 返回是否存在并删除。
    pub fn remove_rule(&self, tool: &str, pattern: &str, session: bool) -> bool {
        if session {
            if let Ok(mut rules) = self.session_rules.lock() {
                let before = rules.len();
                rules.retain(|rule| !(rule.tool == tool && rule.pattern == pattern));
                return rules.len() != before;
            }
            return false;
        }
        if let Ok(mut rules) = self.rules.write() {
            let before = rules.len();
            rules.retain(|rule| !(rule.tool == tool && rule.pattern == pattern));
            return rules.len() != before;
        }
        false
    }

    /// 追加一条显式规则（Deny 规则只能由配置源写入；remember 只产生 Allow）。
    pub fn add_rule(&self, rule: PermissionRule) {
        if let Ok(mut rules) = self.rules.write() {
            rules.push(rule);
        }
    }

    /// 命中当前 (tool, args) 的第一条未过期规则：**会话临时表优先**（用户在
    /// 审批卡上选择的「本会话内允许」压过更早的永久规则；硬拒绝不受影响）。
    fn matching_rule(&self, tool: &str, args: &Value) -> Option<PermissionRule> {
        let now = now_ms();
        if let Ok(session) = self.session_rules.lock() {
            if let Some(rule) = session
                .iter()
                .find(|rule| !rule_expired(rule, now) && rule_matches(rule, tool, args))
            {
                return Some(rule.clone());
            }
        }
        let rules = self.rules.read().ok()?;
        rules
            .iter()
            .find(|rule| !rule_expired(rule, now) && rule_matches(rule, tool, args))
            .cloned()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn rule_expired(rule: &PermissionRule, now_ms: u64) -> bool {
    rule.expires_at_ms
        .is_some_and(|deadline| deadline <= now_ms)
}

/// 规则匹配（语义见 [`PermissionRule`]）。
pub fn rule_matches(rule: &PermissionRule, tool: &str, args: &Value) -> bool {
    if rule.tool != tool {
        return false;
    }
    if rule.pattern == "*" {
        return true;
    }
    if let Some(prefix) = rule.pattern.strip_suffix(" *") {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let token = command.split_whitespace().next().unwrap_or_default();
        return token == prefix;
    }
    if let Some(prefix) = rule.pattern.strip_suffix("/**") {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or_default();
        let normalized = raw.replace('\\', "/");
        let normalized = normalized.trim_start_matches("./");
        return normalized.starts_with(&format!("{prefix}/"));
    }
    false
}

/// 路径归一化：统一分隔符并剥离 workspace 前缀（规则模式存相对形态，便于阅读与迁移）。
fn normalize_rule_path(workspace: &Path, raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let workspace_text = workspace.to_string_lossy().replace('\\', "/");
    let workspace_text = workspace_text.trim_end_matches('/');
    let trimmed = normalized
        .strip_prefix(workspace_text)
        .unwrap_or(&normalized);
    trimmed.trim_start_matches('/').to_string()
}

pub fn resolve_within(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        workspace.join(raw)
    };
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|e| format!("工作区不可访问：{e}"))?;
    let canonical_candidate =
        canonicalize_existing_parent(&candidate).map_err(|e| format!("路径校验失败：{e}"))?;
    if !canonical_candidate.starts_with(&canonical_workspace) {
        return Err(format!("路径位于工作区之外：{path}"));
    }
    Ok(candidate)
}

/// 归一化路径形态：存在则 canonicalize，否则 canonicalize 最近的存在父目录再拼回剩余部件。
pub(crate) fn canonicalize_existing_parent(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return path.canonicalize();
    }
    let mut current = path;
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if current.exists() {
            let mut base = current.canonicalize()?;
            for part in suffix.iter().rev() {
                base.push(part);
            }
            return Ok(base);
        }
        match current.parent() {
            Some(parent) => {
                if let Some(name) = current.file_name() {
                    suffix.push(name.to_os_string());
                }
                current = parent;
            }
            None => return Ok(path.to_path_buf()),
        }
    }
}

/// 归一化比较用的路径文本：Windows 下小写化、统一分隔符，并抹掉 `canonicalize()`
/// 引入的 `\\?\`（verbatim）前缀——否则「已 canonicalize」与「原始」两种形态
/// 字符串前缀比较恒不成立（`\\?\C:\ws\x` vs `C:\ws`），把工作区内路径误判为越界。
fn normalized_path_text(path: &Path) -> String {
    let mut text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        // `\\?\UNC\server\share` 还原为 `\\server\share`，磁盘路径去掉 verbatim 前缀。
        text = match rest.strip_prefix("unc\\") {
            Some(unc) => format!("\\\\{unc}"),
            None => rest.to_string(),
        };
    }
    text
}

/// child 是否位于 root 之内（两侧先归一化形态再比较，Windows 大小写不敏感）。
///
/// 直接 `Path::starts_with` 在 Windows 上会因 verbatim 前缀/大小写/尾部分隔符差异而
/// 恒为 false，把工作区内的路径误判为越界——文件工具与沙箱命令级校验必须用本函数。
pub(crate) fn path_within(child: &Path, root: &Path) -> bool {
    let child_text = normalized_path_text(
        &canonicalize_existing_parent(child).unwrap_or_else(|_| child.to_path_buf()),
    );
    let root_text = normalized_path_text(
        &canonicalize_existing_parent(root).unwrap_or_else(|_| root.to_path_buf()),
    );
    let root_text = root_text.trim_end_matches('\\');
    if root_text.is_empty() {
        return false;
    }
    if child_text == root_text {
        return true;
    }
    child_text
        .strip_prefix(root_text)
        .is_some_and(|rest| rest.starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_only_policy_denies_writes() {
        let policy = Policy::read_only(".");
        let request = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&request), Decision::Deny);
    }

    #[test]
    fn read_only_policy_allows_reads() {
        let policy = Policy::read_only(".");
        let request = policy.evaluate("read_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&request), Decision::Allow);
    }

    #[test]
    fn custom_deny_command_fragment_is_enforced() {
        let mut policy = Policy::new(".");
        policy.add_deny_command("danger-command");
        let request = policy.evaluate(
            "run_command",
            &json!({ "command": "danger-command --force" }),
        );
        assert_eq!(policy.decision(&request), Decision::Deny);
    }

    #[test]
    fn runtime_policy_settings_take_effect_without_rebuilding() {
        let policy = Policy::new(".");
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Ask);

        policy.set_read_only_runtime(true);
        assert_eq!(policy.decision(&write), Decision::Deny);
        policy.set_read_only_runtime(false);
        assert_eq!(policy.decision(&write), Decision::Ask);

        policy.replace_runtime_deny(&["danger-now".to_string()]);
        let denied = policy.evaluate("run_command", &json!({ "command": "danger-now" }));
        assert_eq!(policy.decision(&denied), Decision::Deny);
        policy.replace_runtime_deny(&[]);
        let allowed_to_ask = policy.evaluate("run_command", &json!({ "command": "danger-now" }));
        assert_eq!(policy.decision(&allowed_to_ask), Decision::Ask);
    }

    #[test]
    fn path_within_tolerates_verbatim_prefix_and_case() {
        let temp = std::env::temp_dir();
        let workspace = temp.join("owo-path-within-check");
        let canonical = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.clone());
        // canonicalize 一侧（可能带 \\?\ 前缀）与原始形态混用，仍应判定为工作区内。
        assert!(path_within(&canonical.join("hello.txt"), &workspace));
        assert!(path_within(
            &workspace.join("sub").join("a.txt"),
            &canonical
        ));
        // 前缀相似但不同目录不得误判（compare 需按分隔符边界）。
        assert!(!path_within(
            &temp.join("owo-path-within-check-other"),
            &workspace
        ));
        // 父目录/兄弟路径在工作区之外。
        assert!(!path_within(&temp.join("secret.txt"), &workspace));
    }
}
