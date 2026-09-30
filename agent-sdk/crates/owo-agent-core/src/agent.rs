use crate::audit::AuditLog;
use crate::autoreview::{ReviewVerdict, Reviewer};
use crate::context::{build_system_prompt, load_project_rules};
use crate::error::AgentError;
use crate::gateway::{ChatMessage, ModelOutput, ModelProvider, StreamChunk, TokenUsage};
use crate::injection::sanitize_tool_result;
use crate::permissions::{Approver, Decision, PermissionRequest, Policy};
use crate::session::Session;
use crate::skill::SkillRegistry;
use crate::subagent::SubagentRunner;
use crate::tools::{ToolContext, ToolRegistry};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

const MAX_TOOL_RESULT_CHARS: usize = 50_000;

/// 达到最大回合数后的收尾指令：不再调用工具，强制产出可见结论
/// （审查/分析类任务据此给出结构化报告；信息不足时列出需要用户澄清的问题）。
const WRAP_UP_PROMPT: &str = "你已达到本次任务的最大执行步数上限，现在必须停止调用工具，\
     直接用 Markdown 输出最终结论：1) 已完成的工作与关键发现（审查/分析类任务给出结构化报告：结论、证据、风险）；\
     2) 仍未完成或未验证的部分；3) 如果信息不足，列出需要用户澄清的具体问题。不要再请求任何工具。";

/// 空回答的静默重试次数：第一次空响应直接再问一次（不打扰用户），仍为空才走摘要兜底。
const EMPTY_REPLY_RETRIES: usize = 1;

/// 空回答重试时的追加指令：强制产出可见结论，而不是继续思考或调工具。
const EMPTY_REPLY_RETRY_PROMPT: &str = "（系统提示）你上一条回复没有产生任何可见内容。\
     请不要再调用工具，立即用 Markdown 直接输出：1) 当前已完成的工作与结论；\
     2) 仍未完成或不确定的部分。";

/// 兜底摘要中单条工具动作的参数预览长度上限。
const FALLBACK_ACTION_PREVIEW_CHARS: usize = 90;
/// 兜底摘要中最多列出的工具动作条数（去重后）。
const FALLBACK_ACTION_LIMIT: usize = 20;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub max_turns: usize,
    pub context_limit: usize,
    pub subagent_depth: usize,
    pub token_budget: usize,
    /// 压缩后保留的最近消息条数上限（A4-2：与 keep_recent_tokens 双约束，取交集）。
    pub keep_recent: usize,
    /// 压缩后保留段 token 预算（A4-2）：按条数保留在长工具结果/带图消息场景下
    /// 会显著超出预期，改以 token 为主口径、条数为兜底上限。
    pub keep_recent_tokens: usize,
    pub compaction_enabled: bool,
    /// 模型上下文窗口（tokens，P1-1）：由模型名推导，用于自动预算与诊断。
    pub model_context_window: Option<usize>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 60,
            context_limit: 200,
            subagent_depth: 0,
            token_budget: 60_000,
            keep_recent: 20,
            keep_recent_tokens: 20_000,
            compaction_enabled: true,
            model_context_window: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    ModelCall,
    TokenDelta {
        delta: String,
    },
    /// 深度思考增量（模型 reasoning；不写入对话历史）。
    ReasoningDelta {
        delta: String,
    },
    Compaction {
        summary: String,
    },
    PermissionRequest(PermissionRequest),
    ToolStart {
        id: String,
        tool: String,
        /// 调用入参（随事件下发：前端据此按回合统计改动的文件）。
        #[serde(default)]
        args: Value,
    },
    ToolResult {
        id: String,
        tool: String,
        ok: bool,
        error: Option<String>,
        /// 结果预览（截断）：随事件下发给步骤时间线，避免前端只能看到「已执行 N 个工具」。
        #[serde(default)]
        preview: Option<String>,
    },
    Final {
        text: String,
    },
    /// 任务计划更新（`update_plan` 工具）：UI 渲染步骤进度。
    PlanUpdate {
        steps: Vec<crate::plan_tools::PlanStep>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnOutcome {
    pub final_text: Option<String>,
    pub steps: usize,
    pub events: Vec<TurnEvent>,
    pub prompt: String,
    pub started_at: String,
    pub duration_ms: u64,
    /// 本回合模型 token 用量增量（provider 累计快照差值）。
    #[serde(default)]
    pub usage: TokenUsage,
}

/// Agent 核心：执行循环 + 工具注册表 + 权限策略 + 审计。
pub struct Agent {
    provider: Arc<dyn ModelProvider>,
    /// 工具注册表（RwLock：MCP 服务器热连接/热卸载时无需重建 Agent）。
    registry: Arc<RwLock<ToolRegistry>>,
    /// 插件热卸载：已禁用工具前缀（模型不可见、直接调用被拒）。
    disabled_tool_prefixes: Arc<RwLock<HashSet<String>>>,
    /// MCP 客户端进程生命周期注册表（进程级热卸载/退出清理）。
    mcp_clients: Arc<crate::mcp::McpRegistry>,
    /// 独立审批模型（Auto-review）：Ask 先经审查链，Deny 不打扰用户。
    reviewer: Option<Arc<dyn Reviewer>>,
    policy: Policy,
    audit: Arc<Mutex<AuditLog>>,
    config: AgentConfig,
    skills: SkillRegistry,
    elements: Arc<Mutex<crate::ElementRegistry>>,
    /// Hooks 生命周期扩展点（A2-1）：settings.json 的 `hooks` 灌入；空 = 无 hook。
    /// RwLock：服务运行中（Arc<Agent>）也能重灌（settings 保存后热生效）。
    hooks: std::sync::RwLock<crate::hooks::HookManager>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        registry: ToolRegistry,
        policy: Policy,
        config: AgentConfig,
    ) -> Self {
        Self {
            provider,
            registry: Arc::new(RwLock::new(registry)),
            disabled_tool_prefixes: Arc::new(RwLock::new(HashSet::new())),
            mcp_clients: Arc::new(crate::mcp::McpRegistry::new()),
            reviewer: None,
            policy,
            audit: Arc::new(Mutex::new(AuditLog::default())),
            config,
            skills: SkillRegistry::default(),
            elements: Arc::new(Mutex::new(crate::ElementRegistry::new())),
            hooks: std::sync::RwLock::new(crate::hooks::HookManager::default()),
        }
    }

    pub fn set_skills(&mut self, skills: SkillRegistry) {
        self.skills = skills;
    }

    /// 灌入 hooks 配置（A2-1：settings.json 的 `hooks` 数组；exit 2 = 阻断）。
    /// 快照语义：clone 后释放锁，hook 执行（可达 10s）不阻塞重灌。
    pub fn set_hooks(&self, hooks: crate::hooks::HookManager) {
        if let Ok(mut slot) = self.hooks.write() {
            *slot = hooks;
        }
    }

    /// 当前 hooks 快照（读锁即取即放，避免跨 await 持锁）。
    fn hooks_snapshot(&self) -> crate::hooks::HookManager {
        self.hooks
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 设置独立审批模型（None 表示关闭 Auto-review，恢复纯人工审批）。
    pub fn set_reviewer(&mut self, reviewer: Option<Arc<dyn Reviewer>>) {
        self.reviewer = reviewer;
    }

    /// 当前是否启用 Auto-review。
    pub fn autoreview_enabled(&self) -> bool {
        self.reviewer.is_some()
    }

    /// 注册 MCP 服务器工具（命名空间 `{server}_{tool}`）；热连接，无需重建 Agent。
    pub fn register_mcp_tools(
        &self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<crate::mcp::McpClient>>,
        tools: Vec<crate::mcp::McpTool>,
    ) {
        self.mcp_clients.insert(server_name, Arc::clone(&client));
        if let Ok(mut registry) = self.registry.write() {
            registry.register_mcp_tools(server_name, client, tools);
        }
    }

    /// A2-2：注册 MCP resources/prompts 泛化工具（与 tools 一并热注册）。
    pub fn register_mcp_extras(
        &self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<crate::mcp::McpClient>>,
        resources: Vec<crate::mcp::McpResource>,
        prompts: Vec<crate::mcp::McpPrompt>,
    ) {
        if let Ok(mut registry) = self.registry.write() {
            registry.register_mcp_extras(server_name, client, resources, prompts);
        }
    }

    /// MCP 客户端进程注册表（进程级热卸载/状态查询）。
    pub fn mcp_clients(&self) -> Arc<crate::mcp::McpRegistry> {
        Arc::clone(&self.mcp_clients)
    }

    /// 热连接 MCP 服务器并注册工具（插件启用/热添加）；返回工具数。
    /// A2-2：resources/prompts 非空时一并注册泛化工具（`{server}_read_resource` /
    /// `{server}_get_prompt`）。
    pub async fn connect_mcp_server(
        &self,
        config: &crate::mcp::McpServerConfig,
    ) -> Result<usize, String> {
        let client = crate::mcp::McpClient::connect(config).await?;
        let tools = client.tools();
        let resources = client.resources();
        let prompts = client.prompts();
        let tool_count = tools.len();
        let client = Arc::new(tokio::sync::Mutex::new(client));
        self.register_mcp_tools(&config.name, Arc::clone(&client), tools);
        self.register_mcp_extras(&config.name, client, resources, prompts);
        Ok(tool_count)
    }

    /// 进程级热卸载 MCP 服务器：kill stdio 子进程 + 撤销工具（前缀移除且禁用）。
    /// 返回 false 表示服务器本未连接（幂等，不报错）。
    pub async fn shutdown_mcp_server(&self, name: &str) -> Result<bool, String> {
        if !self.mcp_clients.names().iter().any(|n| n == name) {
            return Ok(false);
        }
        let prefix = crate::tools::mcp_tool_prefix(name);
        self.set_tool_prefix_enabled(&prefix, false);
        self.remove_tools_prefix(&prefix);
        self.mcp_clients.shutdown(name).await?;
        Ok(true)
    }

    /// 关闭全部 MCP 客户端（服务退出前调用，防止遗留 stdio 子进程）。
    pub async fn shutdown_all_mcp(&self) -> Vec<(String, String)> {
        self.mcp_clients.shutdown_all().await
    }

    /// 按前缀撤销工具（插件热卸载）；返回移除数量。
    pub fn remove_tools_prefix(&self, prefix: &str) -> usize {
        self.registry
            .write()
            .map(|mut registry| registry.remove_prefix(prefix))
            .unwrap_or(0)
    }

    /// 插件工具前缀启停（热卸载：模型不可见 + 直接调用被拒，无需重建 Agent）。
    pub fn set_tool_prefix_enabled(&self, prefix: &str, enabled: bool) {
        if let Ok(mut prefixes) = self.disabled_tool_prefixes.write() {
            if enabled {
                prefixes.remove(prefix);
            } else {
                prefixes.insert(prefix.to_string());
            }
        }
    }

    /// 工具名是否命中任一禁用前缀。
    pub fn tool_disabled(&self, name: &str) -> bool {
        self.disabled_tool_prefixes
            .read()
            .map(|prefixes| prefixes.iter().any(|prefix| name.starts_with(prefix)))
            .unwrap_or(false)
    }

    /// 当前模型可见工具：注册表全量减去禁用前缀。
    pub fn visible_tool_specs(&self) -> Vec<crate::tools::ToolSpec> {
        self.registry
            .read()
            .map(|registry| {
                registry
                    .specs()
                    .into_iter()
                    .filter(|spec| !self.tool_disabled(&spec.name))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 设置共享窗口元素注册表（与 HTTP 感知层共用同一 ID 空间）。
    pub fn set_elements(&mut self, elements: Arc<Mutex<crate::ElementRegistry>>) {
        self.elements = elements;
    }

    pub fn elements(&self) -> Arc<Mutex<crate::ElementRegistry>> {
        Arc::clone(&self.elements)
    }

    pub fn skills(&self) -> &SkillRegistry {
        &self.skills
    }

    /// 当前 Agent 配置（只读快照，供诊断/上下文仪表展示）。
    pub fn config(&self) -> AgentConfig {
        self.config.clone()
    }

    pub fn provider(&self) -> Arc<dyn ModelProvider> {
        Arc::clone(&self.provider)
    }

    /// 直呼子代理（CLI `@explore` / `@subagent`）：独立子会话执行，返回最终文本。
    pub async fn run_subagent(
        &self,
        workspace: &std::path::Path,
        model: &str,
        prompt: &str,
        read_only: bool,
    ) -> Result<String, AgentError> {
        let abort = AtomicBool::new(false);
        // 直呼子代理没有可回传到客户端的审批通道：只读模式可以自动放行，
        // 通用模式必须默认拒绝写入/执行，避免子代理绕过主会话审批。
        let approver = crate::permissions::AutoApprover { allow: read_only };
        let runner = SubagentRunner {
            provider: Arc::clone(&self.provider),
            approver: &approver,
            abort: &abort,
            depth: self.config.subagent_depth,
            max_turns: self.config.max_turns,
            model: model.to_string(),
        };
        runner
            .run(workspace, prompt, read_only)
            .await
            .map_err(AgentError::Tool)
    }

    pub fn audit_log(&self) -> Arc<Mutex<AuditLog>> {
        Arc::clone(&self.audit)
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// 运行时追加危险命令片段（热生效；重启后由 settings.deny_commands 恢复）。
    pub fn add_runtime_deny(&self, fragment: impl Into<String>) {
        self.policy.add_runtime_deny(fragment);
    }

    /// 应用运行时权限设置（设置页保存后立即影响下一次工具调用）。
    pub fn apply_policy_settings(&self, read_only: bool, deny_commands: &[String]) {
        self.policy.set_read_only_runtime(read_only);
        self.policy.replace_runtime_deny(deny_commands);
    }

    pub fn registry(&self) -> Arc<RwLock<ToolRegistry>> {
        Arc::clone(&self.registry)
    }

    /// 执行一轮任务。审批经 `approver` 独立决策；`abort` 可随时中止。
    pub async fn run_turn(
        &self,
        session: &mut Session,
        prompt: &str,
        approver: &dyn Approver,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(session, prompt, &[], approver, None, abort, on_event)
            .await
    }

    /// 与 [`Agent::run_turn`] 相同，但额外提供用户提问通道：
    /// `ask_user` 工具经 `questioner` 展示问题并挂起等待回答（`None` 表示无 UI 通道）。
    pub async fn run_turn_with_asker(
        &self,
        session: &mut Session,
        prompt: &str,
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(session, prompt, &[], approver, questioner, abort, on_event)
            .await
    }

    /// 与 [`Agent::run_turn_with_asker`] 相同，但用户消息附带图片
    /// （A1-2 多模态：截图/贴图进主对话上下文，随消息持久化到会话）。
    // 图片/提问/审批/中止/事件回调同为回合执行固有维度，参数数超过 clippy
    // 默认阈值；打包成结构体反而让三处调用点可读性下降，故显式豁免。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_images(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(session, prompt, images, approver, questioner, abort, on_event)
            .await
    }

    /// 预算熔断检查（P2-2）：`OWO_USAGE_TOKEN_BUDGET` / `OWO_USAGE_COST_BUDGET_USD`
    /// 任一超限返回原因；未配置预算时返回 `None`。
    fn budget_stop_reason(&self) -> Option<String> {
        let token_cap = std::env::var("OWO_USAGE_TOKEN_BUDGET")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        let cost_cap = std::env::var("OWO_USAGE_COST_BUDGET_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok());
        if token_cap.is_none() && cost_cap.is_none() {
            return None;
        }
        let input_price = std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let output_price = std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let usage = self.provider.usage_snapshot();
        crate::gateway::budget_violation(&usage, token_cap, cost_cap, input_price, output_price)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_turn_inner(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        let started_at = Utc::now().to_rfc3339();
        let started = std::time::Instant::now();
        let usage_before = self.provider.usage_snapshot();
        // 会话工作区由用户任选（可能不同于服务启动工作区）：按会话派生路径作用域，
        // 否则文件工具会因「路径位于工作区之外」全量被拒、模型被迫用命令行试错。
        let policy = self.policy.scoped_to(session.workspace.clone());
        // 新回合代表从当前历史继续发展，旧的 rewind/undo 分支不能再恢复。
        session.redo_stack.clear();
        session.message_redo_stack.clear();
        let rules = load_project_rules(&session.workspace);
        let mut system = build_system_prompt(session.system_prompt.as_deref(), &rules);
        if !self.skills.list_enabled().is_empty() {
            let mut catalog = vec!["可用技能（通过 use_skill 工具按名调用）：".to_string()];
            for skill in self.skills.list_enabled() {
                catalog.push(format!("- {}：{}", skill.name, skill.description));
            }
            system.push_str("\n\n");
            system.push_str(&catalog.join("\n"));
        }
        let mut messages = vec![ChatMessage::system(system)];
        messages.extend(session.messages.iter().cloned());
        // A1-2：用户消息附带图片（截图/贴图进主对话上下文，随会话持久化）。
        messages.push(ChatMessage::user_with_images(
            prompt.to_string(),
            images.to_vec(),
        ));
        // 存量历史可能带非法序列（压缩切分、中断半截、外部导入）：发请求前归一。
        sanitize_history(&mut messages);
        let tools = self.visible_tool_specs();

        // A2-1 UserPromptSubmit hook：exit 2 = 拒绝本回合（如敏感词门卫、
        // 强制工单号等确定性控制），stderr 回喂模型与用户。
        let hooks = self.hooks_snapshot();
        if !hooks.is_empty() {
            let outcome = hooks
                .run(
                    crate::hooks::HookEvent::UserPromptSubmit,
                    &serde_json::json!({ "prompt": prompt, "session_id": session.id }),
                )
                .await;
            if let crate::hooks::HookOutcome::Blocked(stderr) = outcome {
                self.audit
                    .lock()
                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                    .record(
                        &session.id,
                        "hook_user_prompt_submit",
                        None,
                        Some(false),
                        format!("阻断：{stderr}"),
                    );
                return Err(AgentError::HookBlocked(stderr));
            }
        }

        let mut events = Vec::new();
        let mut final_text = None;
        let mut steps = 0usize;
        // 空回答的静默重试计数（超过上限后走摘要兜底，保证回合必有可见回复）。
        let mut empty_retries = 0usize;
        // P2-2：循环检测（同工具同参反复调用防护）。
        let mut loop_detector = crate::loop_guard::LoopDetector::new();

        for _index in 0..self.config.max_turns {
            if abort.load(Ordering::Relaxed) {
                commit_turn_messages(session, &messages);
                return Err(AgentError::Aborted);
            }
            // P2-2：预算熔断——超限立即终止并给出可见结论，而非事后统计。
            if let Some(reason) = self.budget_stop_reason() {
                let text = format!("已达模型用量预算，任务在第 {steps} 步中止：{reason}");
                messages.push(ChatMessage::assistant_text(text.clone()));
                final_text = Some(text.clone());
                emit(&mut events, on_event, TurnEvent::Final { text });
                break;
            }
            // A2-1 PreCompact hook：通知性质（不阻断——压缩是保护性动作）。
            let hooks = self.hooks_snapshot();
            if !hooks.is_empty() {
                let _ = hooks
                    .run(
                        crate::hooks::HookEvent::PreCompact,
                        &serde_json::json!({
                            "session_id": session.id,
                            "messages": messages.len(),
                            "estimated_tokens": estimate_tokens(&messages),
                        }),
                    )
                    .await;
            }
            let compaction = self.maybe_compact(&mut messages, &session.id).await;
            let summary = match compaction {
                Ok(summary) => summary,
                Err(error) => {
                    commit_turn_messages(session, &messages);
                    return Err(error);
                }
            };
            if let Some(summary) = summary {
                emit(
                    &mut events,
                    on_event,
                    TurnEvent::Compaction {
                        summary: summary.clone(),
                    },
                );
            }
            if messages.len() > self.config.context_limit {
                compact_truncate(&mut messages, self.config.context_limit);
            }

            emit(&mut events, on_event, TurnEvent::ModelCall);
            let on_event_reborrow = &mut *on_event;
            let mut emit_chunk = |chunk: StreamChunk| {
                let event = match chunk {
                    StreamChunk::Content(delta) => TurnEvent::TokenDelta { delta },
                    StreamChunk::Reasoning(delta) => TurnEvent::ReasoningDelta { delta },
                };
                emit(&mut events, on_event_reborrow, event);
            };
            let output = tokio::select! {
                output = self.provider.complete_stream_with_reasoning(&messages, &tools, &mut emit_chunk) => {
                    output.map_err(AgentError::Gateway)
                }
                _ = wait_for_abort(abort) => Err(AgentError::Aborted),
            };
            let output = match output {
                Ok(output) => output,
                Err(error) => {
                    commit_turn_messages(session, &messages);
                    return Err(error);
                }
            };

            match output {
                ModelOutput::Text(text) => {
                    // 空回答（网关截断/模型超载）不再直接失败：先静默重试一次，
                    // 仍为空则用「本回合已执行工具动作摘要」兜底，保证用户总有可见回复，
                    // 而不是只看到「思考过程」后什么都没有。
                    if text.trim().is_empty() {
                        if empty_retries < EMPTY_REPLY_RETRIES {
                            empty_retries += 1;
                            messages.push(ChatMessage::user(EMPTY_REPLY_RETRY_PROMPT.to_string()));
                            continue;
                        }
                        let fallback = synthesize_fallback_reply(
                            &messages,
                            steps,
                            "模型连续返回空回答（可能被网关截断或超载）",
                        );
                        messages.push(ChatMessage::assistant_text(fallback.clone()));
                        final_text = Some(fallback.clone());
                        emit(&mut events, on_event, TurnEvent::Final { text: fallback });
                        break;
                    }
                    messages.push(ChatMessage::assistant_text(text.clone()));
                    final_text = Some(text.clone());
                    emit(&mut events, on_event, TurnEvent::Final { text });
                    break;
                }
                ModelOutput::ToolCalls(calls) => {
                    messages.push(ChatMessage::assistant_tool_calls(calls.clone()));

                    // P2-2：循环检测——同一工具同一参数反复调用时提醒/强制收尾。
                    let mut verdict = crate::loop_guard::LoopVerdict::Ok;
                    for call in &calls {
                        match loop_detector.observe(&call.name, &call.arguments) {
                            crate::loop_guard::LoopVerdict::Ok => {}
                            crate::loop_guard::LoopVerdict::Nudge => {
                                verdict = crate::loop_guard::LoopVerdict::Nudge;
                            }
                            crate::loop_guard::LoopVerdict::WrapUp => {
                                verdict = crate::loop_guard::LoopVerdict::WrapUp;
                                break;
                            }
                        }
                    }
                    match verdict {
                        crate::loop_guard::LoopVerdict::Ok => {}
                        crate::loop_guard::LoopVerdict::Nudge => {
                            messages.push(ChatMessage::user(
                                crate::loop_guard::LOOP_NUDGE_PROMPT.to_string(),
                            ));
                        }
                        crate::loop_guard::LoopVerdict::WrapUp => {
                            messages.push(ChatMessage::user(
                                crate::loop_guard::LOOP_WRAP_UP_PROMPT.to_string(),
                            ));
                            break;
                        }
                    }

                    // —— 阶段 1（必须串行）：权限评估，可能弹审批/等待用户 ——
                    let mut approved_flags = Vec::with_capacity(calls.len());
                    let mut deny_reasons = Vec::with_capacity(calls.len());
                    for call in &calls {
                        if abort.load(Ordering::Relaxed) {
                            commit_turn_messages(session, &messages);
                            return Err(AgentError::Aborted);
                        }
                        // A2-1 PreToolUse hook：exit 2 = 阻断该次调用，stderr 原样
                        // 作为拒绝原因回喂模型（模型可据此换策略），不终止回合。
                        let hooks = self.hooks_snapshot();
                        if !hooks.is_empty() {
                            let outcome = hooks
                                .run(
                                    crate::hooks::HookEvent::PreToolUse,
                                    &serde_json::json!({
                                        "tool": call.name,
                                        "args": call.arguments,
                                        "session_id": session.id,
                                    }),
                                )
                                .await;
                            if let crate::hooks::HookOutcome::Blocked(stderr) = outcome {
                                self.audit
                                    .lock()
                                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                    .record(
                                        &session.id,
                                        "hook_pre_tool_use",
                                        Some(call.name.clone()),
                                        Some(false),
                                        format!("阻断：{stderr}"),
                                    );
                                approved_flags.push(false);
                                deny_reasons.push(format!("hook 阻断：{stderr}"));
                                continue;
                            }
                        }
                        let request = policy.evaluate(&call.name, &call.arguments);
                        let decision = match policy.decision(&request) {
                            Decision::Ask => {
                                // 独立审批模型先于打扰用户（Auto-review）。
                                let verdict = if let Some(reviewer) = &self.reviewer {
                                    let context = session
                                        .messages
                                        .last()
                                        .and_then(|message| message.content.clone());
                                    reviewer.review(&request, context.as_deref()).await
                                } else {
                                    ReviewVerdict::Unknown
                                };
                                match verdict {
                                    ReviewVerdict::Deny => {
                                        self.audit
                                            .lock()
                                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                            .record(
                                                &session.id,
                                                "auto_review",
                                                Some(call.name.clone()),
                                                Some(false),
                                                format!("独立审批模型拒绝：{}", request.reason),
                                            );
                                        Decision::Deny
                                    }
                                    ReviewVerdict::Allow => {
                                        self.audit
                                            .lock()
                                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                            .record(
                                                &session.id,
                                                "auto_review",
                                                Some(call.name.clone()),
                                                Some(true),
                                                "独立审批模型放行".to_string(),
                                            );
                                        Decision::Allow
                                    }
                                    ReviewVerdict::Unknown => {
                                        emit(
                                            &mut events,
                                            on_event,
                                            TurnEvent::PermissionRequest(request.clone()),
                                        );
                                        approver.decide(&request).await
                                    }
                                }
                            }
                            other => other,
                        };
                        let approved = decision == Decision::Allow;
                        self.audit
                            .lock()
                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                            .record(
                                &session.id,
                                "permission",
                                Some(call.name.clone()),
                                Some(approved),
                                request.reason.clone(),
                            );
                        approved_flags.push(approved);
                        deny_reasons.push(request.reason.clone());
                    }

                    // —— 阶段 2：执行。已放行且有并行只读入口的工具并发跑；其余串行，最终按模型给出的顺序回写。 ——
                    let mut outcomes: Vec<(usize, Result<Value, String>)> =
                        Vec::with_capacity(calls.len());
                    let mut serial_indexes: Vec<usize> = Vec::new();
                    let mut parallel_tasks = Vec::new();
                    let workspace_root = session.workspace.clone();

                    for (index, call) in calls.iter().enumerate() {
                        // 句柄只取一次：可并发读判定与并发执行共用，避免两次读注册表之间
                        // 热卸载（插件禁用）导致下面的取用落空（旧实现此处 expect 会 panic，
                        // 整个回合随 SSE 静默断开）。
                        let tool = {
                            let registry = self
                                .registry
                                .read()
                                .map_err(|_| AgentError::Session("工具注册表锁中毒".into()))?;
                            registry.get(&call.name)
                        };
                        let readable = tool
                            .as_ref()
                            .is_some_and(|tool| tool.as_read_only().is_some());
                        let Some(tool) = tool.filter(|_| {
                            approved_flags[index] && readable && !self.tool_disabled(&call.name)
                        }) else {
                            serial_indexes.push(index);
                            continue;
                        };
                        emit(
                            &mut events,
                            on_event,
                            TurnEvent::ToolStart {
                                id: call.id.clone(),
                                tool: call.name.clone(),
                                args: call.arguments.clone(),
                            },
                        );
                        let args = call.arguments.clone();
                        let tool_name = call.name.clone();
                        let ws = workspace_root.clone();
                        let policy_ref = &policy;
                        parallel_tasks.push(async move {
                            let outcome = match tool.as_read_only() {
                                Some(read_only) => {
                                    read_only.run_read_only(&ws, policy_ref, args).await
                                }
                                None => Err(format!("工具不支持并发执行：{tool_name}")),
                            };
                            (index, outcome)
                        });
                    }

                    // P2-2：并行只读工具加并发上限，避免一次放飞过多子进程/网络请求。
                    let semaphore =
                        std::sync::Arc::new(tokio::sync::Semaphore::new(read_tool_permits()));
                    let limited_tasks = parallel_tasks.into_iter().map(|task| {
                        let semaphore = std::sync::Arc::clone(&semaphore);
                        async move {
                            let _permit = semaphore.acquire().await;
                            task.await
                        }
                    });
                    for (index, outcome) in futures::future::join_all(limited_tasks).await {
                        emit(
                            &mut events,
                            on_event,
                            TurnEvent::ToolResult {
                                id: calls[index].id.clone(),
                                tool: calls[index].name.clone(),
                                ok: outcome.is_ok(),
                                error: outcome.as_ref().err().cloned(),
                                preview: tool_preview(&outcome),
                            },
                        );
                        outcomes.push((index, outcome));
                    }

                    for index in serial_indexes {
                        let call = &calls[index];
                        let result = if self.tool_disabled(&call.name) {
                            // 插件热卸载导致工具消失：也要下发终态事件。
                            let error = format!("工具已被禁用（插件热卸载）：{}", call.name);
                            emit(
                                &mut events,
                                on_event,
                                TurnEvent::ToolResult {
                                    id: call.id.clone(),
                                    tool: call.name.clone(),
                                    ok: false,
                                    error: Some(error.clone()),
                                    preview: None,
                                },
                            );
                            Err(error)
                        } else if approved_flags[index] {
                            let workspace = workspace_root.clone();
                            emit(
                                &mut events,
                                on_event,
                                TurnEvent::ToolStart {
                                    id: call.id.clone(),
                                    tool: call.name.clone(),
                                    args: call.arguments.clone(),
                                },
                            );
                            let subagent = SubagentRunner {
                                provider: Arc::clone(&self.provider),
                                approver,
                                abort,
                                depth: self.config.subagent_depth,
                                max_turns: self.config.max_turns,
                                model: session.model.clone(),
                            };
                            // A5-1：fan-out 通道（owned，'static 闭包约束）。
                            let fanout_runner = crate::subagent::FanOutRunner {
                                provider: Arc::clone(&self.provider),
                                workspace: workspace.clone(),
                                model: session.model.clone(),
                                depth: self.config.subagent_depth,
                                max_turns: self.config.max_turns,
                            };
                            let mut ctx = ToolContext {
                                workspace: &workspace,
                                policy: &policy,
                                session,
                                audit: &self.audit,
                                questioner,
                                subagent: Some(subagent),
                                skills: &self.skills,
                                elements: &self.elements,
                                fanout: Some(fanout_runner),
                                abort: Some(abort),
                            };
                            // P2-5：记录计划快照，工具执行后变化即发 PlanUpdate 事件。
                            let plan_before = ctx.session.plan.clone();
                            let tool = self
                                .registry
                                .read()
                                .map_err(|_| AgentError::Session("工具注册表锁中毒".into()))?
                                .get(&call.name);
                            let outcome = match tool {
                                Some(tool) => {
                                    // P2-2：每步超时——超时返回结构化错误，不让整个回合挂起。
                                    let step_timeout = tool_step_timeout();
                                    match tokio::time::timeout(
                                        step_timeout,
                                        tool.run(&mut ctx, call.arguments.clone()),
                                    )
                                    .await
                                    {
                                        Ok(result) => result,
                                        Err(_) => Err(format!(
                                            "工具执行超时（{}s）：{}",
                                            step_timeout.as_secs(),
                                            call.name
                                        )),
                                    }
                                }
                                None => Err(format!("未知工具：{}", call.name)),
                            };
                            emit(
                                &mut events,
                                on_event,
                                TurnEvent::ToolResult {
                                    id: call.id.clone(),
                                    tool: call.name.clone(),
                                    ok: outcome.is_ok(),
                                    error: outcome.as_ref().err().cloned(),
                                    preview: tool_preview(&outcome),
                                },
                            );
                            // P2-5：计划变化时发出计划更新事件（UI 步骤进度）。
                            if session.plan != plan_before {
                                if let Some(plan) = &session.plan {
                                    emit(
                                        &mut events,
                                        on_event,
                                        TurnEvent::PlanUpdate {
                                            steps: plan.steps.clone(),
                                        },
                                    );
                                }
                            }
                            outcome
                        } else {
                            // 权限被拒：同样下发终态事件，否则前端步骤时间线里这一步会永远停在「进行中」。
                            let error = format!("permission denied: {}", deny_reasons[index]);
                            emit(
                                &mut events,
                                on_event,
                                TurnEvent::ToolResult {
                                    id: call.id.clone(),
                                    tool: call.name.clone(),
                                    ok: false,
                                    error: Some(error.clone()),
                                    preview: None,
                                },
                            );
                            Err(error)
                        };
                        outcomes.push((index, result));
                    }

                    // 按模型给出的 tool_call 顺序回写历史与审计（顺序不影响语义，但便于人类阅读和重放）。
                    outcomes.sort_by_key(|(index, _)| *index);
                    for (index, result) in outcomes {
                        let call = &calls[index];
                        let raw_content = match &result {
                            Ok(value) => value.to_string(),
                            Err(error) => format!("工具错误：{error}"),
                        };
                        let content = sanitize_tool_result(
                            &call.name,
                            &truncate_tool_result(&raw_content, MAX_TOOL_RESULT_CHARS),
                        );
                        messages.push(ChatMessage::tool(call.id.clone(), content.clone()));
                        self.audit
                            .lock()
                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                            .record(
                                &session.id,
                                "tool_call",
                                Some(call.name.clone()),
                                None,
                                content.clone(),
                            );
                        steps += 1;
                    }
                }
            }
        }

        if final_text.is_none() {
            // 步数耗尽不能只甩一句「达到最大回合数」：再补一次不带工具的收尾总结，
            // 保证回合一定有可见结论（审查/分析类任务据此产出报告），
            // 而不是让用户看到「思考完就停住」。
            if abort.load(Ordering::Relaxed) {
                commit_turn_messages(session, &messages);
                return Err(AgentError::Aborted);
            }
            emit(&mut events, on_event, TurnEvent::ModelCall);
            let mut wrap_messages = messages.clone();
            wrap_messages.push(ChatMessage::user(WRAP_UP_PROMPT.to_string()));
            let on_event_reborrow = &mut *on_event;
            let mut emit_chunk = |chunk: StreamChunk| {
                let event = match chunk {
                    StreamChunk::Content(delta) => TurnEvent::TokenDelta { delta },
                    StreamChunk::Reasoning(delta) => TurnEvent::ReasoningDelta { delta },
                };
                emit(&mut events, on_event_reborrow, event);
            };
            let wrap_up = self
                .provider
                .complete_stream_with_reasoning(&wrap_messages, &[], &mut emit_chunk)
                .await;
            // 收尾总结同样不允许「空手而归」：模型没产出内容（或调用失败）时，
            // 用本回合已执行的工具动作摘要兜底——回合必须以可见结论结束。
            let text = match wrap_up {
                Ok(ModelOutput::Text(text)) if !text.trim().is_empty() => text,
                Ok(_) => synthesize_fallback_reply(
                    &messages,
                    steps,
                    &format!(
                        "达到最大回合数（{}）且收尾总结未产出内容",
                        self.config.max_turns
                    ),
                ),
                Err(error) => synthesize_fallback_reply(
                    &messages,
                    steps,
                    &format!(
                        "达到最大回合数（{}）且收尾总结调用失败：{error}",
                        self.config.max_turns
                    ),
                ),
            };
            messages.push(ChatMessage::assistant_text(text.clone()));
            final_text = Some(text.clone());
            emit(&mut events, on_event, TurnEvent::Final { text });
        }
        commit_turn_messages(session, &messages);
        // A2-1 Stop hook：回合结束通知（通知性质；finally 类动作如测试/通知）。
        let hooks = self.hooks_snapshot();
        if !hooks.is_empty() {
            let _ = hooks
                .run(
                    crate::hooks::HookEvent::Stop,
                    &serde_json::json!({
                        "session_id": session.id,
                        "stop_reason": final_text.as_deref().map(|t| t.chars().take(120).collect::<String>()),
                    }),
                )
                .await;
        }
        let usage = self.provider.usage_snapshot().saturating_sub(&usage_before);
        Ok(TurnOutcome {
            final_text,
            steps,
            events,
            prompt: prompt.to_string(),
            started_at,
            duration_ms: started.elapsed().as_millis() as u64,
            usage,
        })
    }

    /// 当估算 token 超过预算时，用模型把旧历史压缩为摘要（保留最近消息）。
    async fn maybe_compact(
        &self,
        messages: &mut Vec<ChatMessage>,
        session_id: &str,
    ) -> Result<Option<String>, AgentError> {
        if !self.config.compaction_enabled || estimate_tokens(messages) <= self.config.token_budget
        {
            return Ok(None);
        }
        // 切点（A4-2）：保留段按 token 预算从尾部选取，同时受 keep_recent 条数
        // 上限约束；再用 align_keep_start 对齐 tool 群组——否则保留段以孤立
        // tool 消息开头，模型侧直接 400（曾实测 DeepSeek 拒绝整轮）。
        let tail_start = compute_keep_start(
            messages,
            self.config.keep_recent,
            self.config.keep_recent_tokens,
        );
        let head_end = align_keep_start(messages, tail_start);
        if head_end < 4 {
            return Ok(None);
        }
        let head = messages[1..head_end].to_vec();
        // A4-1 决策骨架：要求结构化小节，压缩后模型仍保有「为什么这么做/踩过什么坑」，
        // 纯散文摘要会把这些丢光，导致压缩后重蹈覆辙或丢失关键路径上下文。
        let prompt = format!(
            "请把以下 Agent 会话历史压缩成一份「决策骨架」摘要，用以下 Markdown 小节组织\
             （没有内容的小节写「无」，总长度不超过 800 字，只记录事实、不要编造）：\n\n\
             ## 已完成\n- 已执行的关键动作与结果（文件/命令/验证结论）\n\n\
             ## 关键决策\n- 做出的重要选择及理由（为什么这么做、放弃了什么）\n\n\
             ## 失败与教训\n- 走过的弯路、报错及规避方式\n\n\
             ## 未完成\n- 尚未完成的任务与下一步\n\n\
             ## 上下文备忘\n- 后续工作必需的文件路径、符号名、约定、用户偏好\n\n\
             ----------\n\n{}",
            serde_json::to_string(&head).unwrap_or_else(|_| "[]".to_string())
        );
        let summary = match self
            .provider
            .complete(&[ChatMessage::user(prompt)], &[])
            .await
        {
            Ok(crate::gateway::ModelOutput::Text(text)) => text,
            Ok(_) | Err(_) => {
                // 压缩失败兜底（A4-3）：按 token 硬裁剪历史，保证本回合仍可发送，
                // 而不是带着超预算历史直接撞模型 400。返回 Some 让前端收到可见提示。
                let before = messages.len();
                compact_truncate_to_budget(messages, self.config.token_budget / 2);
                self.audit
                    .lock()
                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                    .record(
                        session_id,
                        "compaction_fallback",
                        None,
                        None,
                        format!(
                            "模型压缩失败，按预算硬裁剪历史：{before} → {} 条",
                            messages.len().saturating_sub(1)
                        ),
                    );
                return Ok(Some(
                    "模型压缩失败，已硬裁剪最近历史以保持在上下文预算内".to_string(),
                ));
            }
        };
        let mut compacted = vec![messages[0].clone()];
        compacted.push(ChatMessage::system(format!(
            "历史摘要（已压缩）：\n{summary}"
        )));
        compacted.extend(messages[head_end..].to_vec());
        *messages = compacted;
        self.audit
            .lock()
            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
            .record(
                session_id,
                "compaction",
                None,
                None,
                format!("压缩 {} 条历史消息", head.len()),
            );
        Ok(Some(summary))
    }
}

/// 合成兜底回复：模型在回合结束时没有产出任何内容（空回答/收尾失败）时，
/// 把本回合已执行的工具动作整理成可读摘要，保证用户总能得到明确结论，
/// 而不是只看到一段越来越长的「思考过程」后什么都没有。
fn synthesize_fallback_reply(messages: &[ChatMessage], steps: usize, reason: &str) -> String {
    let mut actions: Vec<String> = Vec::new();
    for message in messages {
        let Some(calls) = &message.tool_calls else {
            continue;
        };
        for call in calls {
            let preview = tool_call_preview(&call.arguments);
            let line = if preview.is_empty() {
                format!("- `{}`", call.name)
            } else {
                format!("- `{}`：{preview}", call.name)
            };
            if !actions.contains(&line) {
                actions.push(line);
            }
        }
    }
    let shown = actions.len().min(FALLBACK_ACTION_LIMIT);
    let mut body = String::new();
    body.push_str("> ⚠️ 本回合模型没有产出正式回答（");
    body.push_str(reason);
    body.push_str("）。以下为系统自动整理的工作摘要，供你确认或让我继续。\n\n");
    if actions.is_empty() {
        body.push_str("**本轮没有执行任何工具操作，也没有产出文本内容。**\n\n");
    } else {
        body.push_str(&format!(
            "**本回合共执行 {steps} 步工具操作（列出前 {shown} 条）：**\n\n"
        ));
        for line in actions.iter().take(shown) {
            body.push_str(line);
            body.push('\n');
        }
        if actions.len() > shown {
            body.push_str(&format!("- …（其余 {} 条已省略）\n", actions.len() - shown));
        }
        body.push('\n');
    }
    body.push_str("你可以直接回复「继续」让我接着完成剩余部分，或指出需要调整的地方。");
    body
}

/// 工具调用参数摘要：优先取路径/命令等最具信息量的字段，截断到预览长度上限。
fn tool_call_preview(arguments: &Value) -> String {
    const KEYS: [&str; 6] = ["path", "command", "file", "pattern", "query", "url"];
    let raw = KEYS
        .iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .or_else(|| {
            arguments
                .as_object()
                .and_then(|map| map.values().find_map(Value::as_str))
        })
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut preview: String = raw.chars().take(FALLBACK_ACTION_PREVIEW_CHARS).collect();
    if raw.chars().count() > FALLBACK_ACTION_PREVIEW_CHARS {
        preview.push('…');
    }
    preview.replace('\n', " ")
}

fn commit_turn_messages(session: &mut Session, messages: &[ChatMessage]) {
    session.messages = messages.iter().skip(1).cloned().collect();
    session.updated_at = Utc::now().to_rfc3339();
}

async fn wait_for_abort(abort: &AtomicBool) {
    while !abort.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn truncate_tool_result(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut truncated: String = content.chars().take(max_chars).collect();
    truncated.push_str("\n[工具输出已截断]");
    truncated
}

/// 步骤时间线的结果预览上限：够看清这步做了什么，又不会把 SSE 帧撑爆。
const TOOL_PREVIEW_CHARS: usize = 1600;

/// 工具结果预览（随 ToolResult 事件下发给前端 chip 展开区）。
/// 纯展示用途：写回模型上下文的净化仍由 `sanitize_tool_result` 负责。
fn tool_preview(outcome: &Result<Value, String>) -> Option<String> {
    let text = match outcome {
        Ok(value) => value.to_string(),
        Err(error) => format!("工具错误：{error}"),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_tool_result(trimmed, TOOL_PREVIEW_CHARS))
}

/// 单张图片的保守 token 开销（A1-2）：Anthropic/OpenAI 的视觉 token 随分辨率
/// 浮动（数百到数千），按 1100 中位值计入预算，防止带图消息把 token 估算打穿。
pub const IMAGE_TOKEN_ESTIMATE: usize = 1_100;

/// 消息序列 token 估算：真实计数器（tiktoken `cl100k_base`，不可用时启发式兜底）
/// + 每条消息固定开销 + 图片固定开销。
///
/// P1-1：替换原「字符数 / 2 + 4」的折半估算——中文场景下旧公式误差 2~3 倍，
/// 会导致压缩触发过晚（超上下文 → 400）或过早（丢关键历史）。
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    let counter = crate::tokenizer::default_counter();
    messages
        .iter()
        .map(|message| {
            counter.count(message.content.as_deref().unwrap_or_default())
                + crate::tokenizer::MESSAGE_OVERHEAD
                + message.images.len() * IMAGE_TOKEN_ESTIMATE
        })
        .sum()
}

/// 单步工具执行超时（P2-2）：默认 120s，`OWO_TOOL_TIMEOUT_SECS` 覆盖（>0）。
fn tool_step_timeout() -> std::time::Duration {
    let secs = std::env::var("OWO_TOOL_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120);
    std::time::Duration::from_secs(secs)
}

/// 并行只读工具并发上限（P2-2）：默认 4，`OWO_CONCURRENT_READ_TOOLS` 覆盖（>0）。
fn read_tool_permits() -> usize {
    std::env::var("OWO_CONCURRENT_READ_TOOLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(4)
}

fn emit(
    events: &mut Vec<TurnEvent>,
    on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    event: TurnEvent,
) {
    on_event(&event);
    events.push(event);
}

/// 把「保留段起点」对齐到合法消息边界：起点若落在 tool 结果群组内，
/// 连同该群组的 assistant(tool_calls) 一起保留；群组前没有对应
/// assistant(tool_calls) 时跳过整个群组。
///
/// 模型侧（DeepSeek 等）会对「tool 消息前面缺 tool_calls」的序列直接返 400，
/// 压缩/裁剪切分历史时必须用它对齐切点。
fn align_keep_start(messages: &[ChatMessage], start: usize) -> usize {
    if start >= messages.len() || messages[start].role != "tool" {
        return start;
    }
    let mut group_start = start;
    while group_start > 1 && messages[group_start - 1].role == "tool" {
        group_start -= 1;
    }
    if group_start > 1
        && messages[group_start - 1].role == "assistant"
        && messages[group_start - 1].tool_calls.is_some()
    {
        return group_start - 1;
    }
    let mut after = start;
    while after < messages.len() && messages[after].role == "tool" {
        after += 1;
    }
    after
}

/// 压缩保留段起点（A4-2）：从尾部往前累计 token，受 `token_budget`（token）
/// 与 `max_messages`（条数兜底）双约束。最新一条消息无条件纳入（避免空保留段）。
/// 返回值 ≥ 1（跳过 system，交给 align_keep_start 对齐 tool 群组）。
fn compute_keep_start(messages: &[ChatMessage], max_messages: usize, token_budget: usize) -> usize {
    let mut acc = 0usize;
    let mut start = messages.len();
    for index in (1..messages.len()).rev() {
        let tokens = estimate_tokens(&messages[index..index + 1]);
        // 首条无条件保留；后续条目超预算即停。
        if start < messages.len() && acc + tokens > token_budget {
            break;
        }
        acc += tokens;
        start = index;
        if messages.len() - start >= max_messages {
            break;
        }
    }
    start.max(1)
}

/// 归一历史序列：丢弃孤立的 tool 消息，为缺失结果的 assistant(tool_calls)
/// 补占位结果。历史可能因压缩切分、回合中断、外部导入等原因带上这类脏数据，
/// 每次请求前清理，保证发出的序列始终合法（否则模型侧直接 400 拒绝整轮）。
fn sanitize_history(messages: &mut Vec<ChatMessage>) {
    fn flush_pending(pending: &mut Vec<String>, cleaned: &mut Vec<ChatMessage>) {
        for id in pending.drain(..) {
            cleaned.push(ChatMessage::tool(
                id,
                "工具结果缺失（该回合被中断或未完成）".to_string(),
            ));
        }
    }

    let mut cleaned: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut pending: Vec<String> = Vec::new();
    for message in messages.drain(..) {
        match message.role.as_str() {
            "tool" => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                if let Some(position) = pending.iter().position(|call_id| *call_id == id) {
                    pending.remove(position);
                    cleaned.push(message);
                }
                // 无配对（孤立 tool）：丢弃
            }
            "assistant" => {
                flush_pending(&mut pending, &mut cleaned);
                if let Some(calls) = &message.tool_calls {
                    pending.extend(calls.iter().map(|call| call.id.clone()));
                }
                cleaned.push(message);
            }
            _ => {
                flush_pending(&mut pending, &mut cleaned);
                cleaned.push(message);
            }
        }
    }
    flush_pending(&mut pending, &mut cleaned);
    *messages = cleaned;
}

fn compact_truncate(messages: &mut Vec<ChatMessage>, limit: usize) {
    if messages.len() <= limit {
        return;
    }
    let keep = limit.saturating_sub(1);
    let tail_start = align_keep_start(messages, messages.len().saturating_sub(keep));
    let mut tail = messages[tail_start..].to_vec();
    let system = messages[0].clone();
    tail.insert(0, system);
    *messages = tail;
}

/// 按 token 预算硬裁剪历史（A4-3 兜底）：从尾部保留尽量多的消息（对齐 tool
/// 群组切点），使 `estimate_tokens` 回到 `budget` 内。
///
/// 与条数版 [`compact_truncate`] 的区别：模型上下文是 token 硬约束，压缩模型
/// 调用失败时条数检查（`messages.len() > context_limit`）挡不住 token 超限。
fn compact_truncate_to_budget(messages: &mut Vec<ChatMessage>, budget: usize) {
    let counter = crate::tokenizer::default_counter();
    let token_of = |message: &ChatMessage| {
        counter.count(message.content.as_deref().unwrap_or_default())
            + crate::tokenizer::MESSAGE_OVERHEAD
    };
    // system（messages[0]）必保留；从尾部往前累计，找出预算内可保留的 tail。
    let mut acc = messages.first().map(&token_of).unwrap_or(0);
    let mut keep = 0usize;
    for message in messages.iter().skip(1).rev() {
        let tokens = token_of(message);
        if acc + tokens > budget && keep > 0 {
            break;
        }
        acc += tokens;
        keep += 1;
    }
    if keep == 0 || keep >= messages.len() {
        return;
    }
    let tail_start = align_keep_start(messages, messages.len() - keep);
    // 对齐可能把 tail 推到群组之后甚至越界：越界时不裁（保守，不破坏序列）。
    if tail_start >= messages.len() {
        return;
    }
    let mut tail = messages[tail_start..].to_vec();
    let system = messages[0].clone();
    tail.insert(0, system);
    *messages = tail;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_counts_cjk_realistically() {
        let messages = vec![
            ChatMessage::system("规则".to_string()),
            ChatMessage::user("你好，请帮我总结这段代码".to_string()),
            ChatMessage::assistant_text("好的。".to_string()),
        ];
        let total = estimate_tokens(&messages);
        // P1-1 口径：中文 ≈1 token/字（cl100k 实测区间），3 条共 17 字 + 12 开销。
        // 旧的「字符数/2」会给出 ~20 的低估；真实计数应明显更高。
        assert!(
            (22..=45).contains(&total),
            "估算 token {total} 应在中文真实区间（旧公式低估）"
        );
    }

    #[test]
    fn empty_messages_cost_zero() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    #[test]
    fn compact_truncate_keeps_system_and_recent_tail() {
        let mut messages = vec![ChatMessage::system("系统".to_string())];
        for index in 0..10 {
            messages.push(ChatMessage::user(format!("消息{index}")));
        }
        compact_truncate(&mut messages, 4);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "system");
        assert!(messages
            .iter()
            .any(|message| message.content.as_deref() == Some("消息9")));
        assert!(messages
            .iter()
            .any(|message| message.content.as_deref() == Some("消息7")));
    }

    #[test]
    fn compact_truncate_keeps_tool_call_and_results_together() {
        let mut messages = vec![
            ChatMessage::system("系统".to_string()),
            ChatMessage::user("旧请求".to_string()),
            ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
                id: "call-1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "a.txt" }),
            }]),
            ChatMessage::tool("call-1".to_string(), "结果".to_string()),
            ChatMessage::user("继续".to_string()),
            ChatMessage::assistant_text("好的".to_string()),
        ];

        compact_truncate(&mut messages, 4);

        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].tool_calls.is_some());
        assert_eq!(messages[2].role, "tool");
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call-1"));
    }

    #[test]
    fn compact_truncate_to_budget_brings_tokens_within_budget() {
        // 每条消息制造足够 token（长中文文本），预算设小，裁剪后必须回到预算内。
        let mut messages = vec![ChatMessage::system("系统提示词".to_string())];
        for index in 0..40 {
            messages.push(ChatMessage::user(format!(
                "历史消息{index}，{}",
                "内容".repeat(200)
            )));
        }
        let budget = 2_000usize;
        compact_truncate_to_budget(&mut messages, budget);
        assert!(
            estimate_tokens(&messages) <= budget,
            "裁剪后 token（{}）应回到预算 {budget} 内",
            estimate_tokens(&messages)
        );
        // system 必保留；尾部最近消息必保留。
        assert_eq!(messages[0].role, "system");
        assert!(messages.len() >= 2, "至少应保留一条业务消息");
        let latest = format!("历史消息39，{}", "内容".repeat(200));
        assert!(messages
            .iter()
            .any(|message| message.content.as_deref() == Some(latest.as_str())));
    }

    #[test]
    fn compact_truncate_to_budget_keeps_tool_group_together() {
        // tail 起点落在 tool 群组内时，应回退到 assistant(tool_calls) 一起保留。
        let mut messages = vec![ChatMessage::system("系统".to_string())];
        for index in 0..20 {
            messages.push(ChatMessage::user(format!(
                "旧请求{index}，{}",
                "填充".repeat(300)
            )));
        }
        messages.push(ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "call-9".to_string(),
            name: "grep".to_string(),
            arguments: serde_json::json!({ "pattern": "x" }),
        }]));
        messages.push(ChatMessage::tool("call-9".to_string(), "结果".to_string()));
        messages.push(ChatMessage::user("最新请求".to_string()));
        messages.push(ChatMessage::assistant_text("回答".to_string()));

        compact_truncate_to_budget(&mut messages, 1_500usize);

        assert_eq!(messages[0].role, "system");
        // 裁剪后序列仍合法：不出现「孤立 tool 消息开头」。
        if let Some(position) = messages.iter().position(|m| m.role == "tool") {
            let previous = &messages[position - 1];
            assert_eq!(previous.role, "assistant");
            assert!(previous.tool_calls.is_some());
        }
    }

    #[test]
    fn compact_truncate_to_budget_noop_when_within_budget() {
        let mut messages = vec![
            ChatMessage::system("系统".to_string()),
            ChatMessage::user("第一条".to_string()),
            ChatMessage::assistant_text("回复".to_string()),
        ];
        let roles_before: Vec<String> = messages.iter().map(|m| m.role.clone()).collect();
        let contents_before: Vec<String> =
            messages.iter().map(|m| m.content.clone().unwrap_or_default()).collect();
        compact_truncate_to_budget(&mut messages, 100_000);
        let roles_after: Vec<String> = messages.iter().map(|m| m.role.clone()).collect();
        let contents_after: Vec<String> =
            messages.iter().map(|m| m.content.clone().unwrap_or_default()).collect();
        assert_eq!(roles_after, roles_before);
        assert_eq!(contents_after, contents_before);
    }

    #[test]
    fn tool_result_is_bounded_without_splitting_unicode() {
        let result = truncate_tool_result(&"中".repeat(10), 3);
        assert!(result.starts_with("中中中"));
        assert!(result.contains("工具输出已截断"));
    }

    #[test]
    fn compute_keep_start_respects_token_budget() {
        // 每条消息制造足够 token：token 预算生效时保留段应远小于全部条数。
        let mut messages = vec![ChatMessage::system("系统".to_string())];
        for index in 0..30 {
            messages.push(ChatMessage::user(format!("消息{index}，{}", "内容".repeat(400))));
        }
        // 预算足够大时不受 token 约束，条数上限也不触发 → 全保留。
        let all_kept = compute_keep_start(&messages, 100, 1_000_000);
        assert_eq!(all_kept, 1);
        // 小预算：保留段 token 收敛到预算附近（首条无条件计入，允许少量超出）。
        let start = compute_keep_start(&messages, 100, 3_000);
        let kept = estimate_tokens(&messages[start..]);
        assert!(
            kept <= 3_000 + estimate_tokens(&messages[1..2]),
            "保留段 token（{kept}）应不超过预算+首条"
        );
        assert!(messages.len() - start < 30, "token 预算应限制保留条数");
    }

    #[test]
    fn compute_keep_start_respects_message_cap() {
        // 短消息 + 大 token 预算：条数上限（keep_recent）仍生效。
        let mut messages = vec![ChatMessage::system("系统".to_string())];
        for index in 0..30 {
            messages.push(ChatMessage::user(format!("短消息{index}")));
        }
        let start = compute_keep_start(&messages, 3, 1_000_000);
        assert_eq!(messages.len() - start, 3);
    }

    #[test]
    fn compute_keep_start_always_keeps_latest_message() {
        // 预算小于单条消息：至少保留最新一条（避免空保留段）。
        let messages = vec![
            ChatMessage::system("系统".to_string()),
            ChatMessage::user(format!("最新请求，{}", "内容".repeat(500))),
        ];
        let start = compute_keep_start(&messages, 20, 10);
        assert_eq!(start, 1, "最新消息必须保留");
    }

    #[test]
    fn align_keep_start_pulls_in_tool_call_message() {
        // 切点落在 tool 群组内：应回退到 assistant(tool_calls)，让调用与结果
        // 同进同出（否则保留段以孤立 tool 开头，模型侧直接 400）。
        let messages = vec![
            ChatMessage::system("系统".to_string()),
            ChatMessage::user("请求".to_string()),
            ChatMessage::assistant_tool_calls(vec![
                crate::gateway::ToolCall {
                    id: "c1".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({ "path": "a.txt" }),
                },
                crate::gateway::ToolCall {
                    id: "c2".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({ "path": "b.txt" }),
                },
            ]),
            ChatMessage::tool("c1".to_string(), "结果一".to_string()),
            ChatMessage::tool("c2".to_string(), "结果二".to_string()),
        ];
        // 居中切分（第 4 条 = c2 的结果）与首条切分（第 3 条 = c1 的结果）都回退到 assistant。
        assert_eq!(align_keep_start(&messages, 4), 2);
        assert_eq!(align_keep_start(&messages, 3), 2);
        // 切点在群组之外：不动。
        assert_eq!(align_keep_start(&messages, 5), 5);
    }

    #[test]
    fn align_keep_start_skips_orphan_tool_group() {
        // 群组前面没有 assistant(tool_calls)（脏历史）：跳过整个群组，
        // 保留段从群组之后的 user 开始。
        let messages = vec![
            ChatMessage::system("系统".to_string()),
            ChatMessage::user("请求".to_string()),
            ChatMessage::tool("孤儿".to_string(), "结果".to_string()),
            ChatMessage::user("继续".to_string()),
        ];
        assert_eq!(align_keep_start(&messages, 2), 3);
    }

    #[test]
    fn sanitize_history_drops_orphan_tool_and_fills_missing_results() {
        let mut messages = vec![
            ChatMessage::system("系统".to_string()),
            // 孤立 tool（压缩摘要后遗留）：丢弃。
            ChatMessage::tool("orphan-1".to_string(), "孤儿结果".to_string()),
            ChatMessage::user("请求".to_string()),
            // 配对完整：原样保留。
            ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
                id: "keep-1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "a.txt" }),
            }]),
            ChatMessage::tool("keep-1".to_string(), "正常结果".to_string()),
            // 缺结果（回合中断半截提交）：补占位结果。
            ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
                id: "lost-1".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({ "path": "b.txt" }),
            }]),
            ChatMessage::user("新回合".to_string()),
        ];
        sanitize_history(&mut messages);

        assert_eq!(messages[0].role, "system");
        assert!(!messages
            .iter()
            .any(|message| message.tool_call_id.as_deref() == Some("orphan-1")));
        assert!(messages
            .iter()
            .any(|message| message.tool_call_id.as_deref() == Some("keep-1")));
        // 补的占位结果必须紧跟在对应 assistant(tool_calls) 之后。
        let lost_index = messages
            .iter()
            .position(|message| {
                message
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| calls.iter().any(|call| call.id == "lost-1"))
            })
            .expect("lost-1 的 assistant 消息应保留");
        assert_eq!(messages[lost_index + 1].role, "tool");
        assert_eq!(
            messages[lost_index + 1].tool_call_id.as_deref(),
            Some("lost-1")
        );
    }

    /// 脚本化 Provider：`EmptyText` 恒返回空文本（模拟网关截断）；
    /// `ToolCallsThenWrapUp` 恒返回未知工具调用，直到收尾调用（tools 为空）才返回文本。
    enum ScriptedBehavior {
        EmptyText,
        ToolCallsThenWrapUp,
    }

    struct ScriptedProvider {
        behave: ScriptedBehavior,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ModelProvider for ScriptedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[crate::tools::ToolSpec],
        ) -> Result<ModelOutput, String> {
            Ok(ModelOutput::Text("摘要".to_string()))
        }

        async fn complete_stream_with_reasoning(
            &self,
            _messages: &[ChatMessage],
            tools: &[crate::tools::ToolSpec],
            on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
        ) -> Result<ModelOutput, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.behave {
                ScriptedBehavior::EmptyText => Ok(ModelOutput::Text(String::new())),
                ScriptedBehavior::ToolCallsThenWrapUp => {
                    if tools.is_empty() {
                        on_chunk(StreamChunk::Content("最终结论".to_string()));
                        Ok(ModelOutput::Text("最终结论".to_string()))
                    } else {
                        Ok(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
                            id: format!("call-{}", self.calls.load(Ordering::Relaxed)),
                            name: "noop_tool".to_string(),
                            arguments: serde_json::json!({}),
                        }]))
                    }
                }
            }
        }
    }

    fn scripted_agent(behave: ScriptedBehavior, max_turns: usize) -> Agent {
        let provider: Arc<dyn ModelProvider> = Arc::new(ScriptedProvider {
            behave,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let config = AgentConfig {
            max_turns,
            ..AgentConfig::default()
        };
        Agent::new(
            provider,
            ToolRegistry::default(),
            Policy::new(std::env::temp_dir()),
            config,
        )
    }

    #[tokio::test]
    async fn empty_final_answer_falls_back_to_visible_reply() {
        // 空回答不能只留「思考过程」：先自动重试一次，仍为空则用兜底摘要收尾，
        // 保证每个回合都有可见回复（对齐「不管怎么样都要给用户回复」的产品要求）。
        let agent = scripted_agent(ScriptedBehavior::EmptyText, 4);
        let mut session = Session::new(std::env::temp_dir(), "test-model", None);
        let approver = crate::permissions::AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let mut finals = Vec::new();
        let outcome = agent
            .run_turn(
                &mut session,
                "审查这个仓库",
                &approver,
                &abort,
                &mut |event| {
                    if let TurnEvent::Final { text } = event {
                        finals.push(text.clone());
                    }
                },
            )
            .await
            .expect("空回答也必须以兜底回复正常结束");
        let final_text = outcome.final_text.unwrap_or_default();
        assert!(!final_text.trim().is_empty(), "兜底回复不能为空");
        assert!(
            final_text.contains("没有产出正式回答"),
            "应说明兜底原因：{final_text}"
        );
        assert_eq!(finals.len(), 1, "应恰好下发一次 Final");
        assert_eq!(finals[0], final_text);
        // 兜底回复入历史：用户恢复会话后仍能看到这条结论（含原因说明）。
        assert!(session.messages.iter().any(|message| {
            message.role == "assistant" && message.content.as_deref() == Some(final_text.as_str())
        }));
        assert!(session
            .messages
            .iter()
            .any(|message| message.content.as_deref() == Some("审查这个仓库")));
    }

    #[tokio::test]
    async fn ask_user_tool_resumes_turn_with_user_answer() {
        // ask_user 全链路：模型发起提问 → Questioner 送回用户答案 → 工具结果回填 →
        // 模型基于答案给出结论（「拿不准就问用户，问完继续把活干完」）。
        struct AskThenAnswerProvider {
            calls: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl ModelProvider for AskThenAnswerProvider {
            async fn complete(
                &self,
                _messages: &[ChatMessage],
                _tools: &[crate::tools::ToolSpec],
            ) -> Result<ModelOutput, String> {
                Ok(ModelOutput::Text("摘要".to_string()))
            }

            async fn complete_stream_with_reasoning(
                &self,
                messages: &[ChatMessage],
                _tools: &[crate::tools::ToolSpec],
                on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
            ) -> Result<ModelOutput, String> {
                let index = self.calls.fetch_add(1, Ordering::Relaxed);
                if index == 0 {
                    return Ok(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
                        id: "ask-1".to_string(),
                        name: "ask_user".to_string(),
                        arguments: serde_json::json!({ "question": "用 A 还是 B？" }),
                    }]));
                }
                let answer_seen = messages.iter().any(|message| {
                    message.role == "tool"
                        && message
                            .content
                            .as_deref()
                            .unwrap_or_default()
                            .contains("就用 B")
                });
                let text = if answer_seen {
                    "已按 B 方案完成"
                } else {
                    "未拿到用户回答"
                };
                on_chunk(StreamChunk::Content(text.to_string()));
                Ok(ModelOutput::Text(text.to_string()))
            }
        }

        struct Answerer;

        #[async_trait::async_trait]
        impl crate::question::Questioner for Answerer {
            async fn ask(
                &self,
                question: &crate::question::UserQuestion,
            ) -> Option<crate::question::QuestionAnswer> {
                assert!(question.question.contains("A 还是 B"), "问题应原样透传");
                Some(crate::question::QuestionAnswer {
                    question_id: question.question_id.clone(),
                    answer: "就用 B".to_string(),
                })
            }
        }

        let provider: Arc<dyn ModelProvider> = Arc::new(AskThenAnswerProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let agent = Agent::new(
            provider,
            ToolRegistry::default(),
            Policy::new(std::env::temp_dir()),
            AgentConfig::default(),
        );
        let mut session = Session::new(std::env::temp_dir(), "test-model", None);
        let approver = crate::permissions::AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let answerer = Answerer;
        let outcome = agent
            .run_turn_with_asker(
                &mut session,
                "开始实现",
                &approver,
                Some(&answerer),
                &abort,
                &mut |_| {},
            )
            .await
            .expect("提问后回合应正常结束");
        assert_eq!(outcome.final_text.as_deref(), Some("已按 B 方案完成"));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            TurnEvent::ToolResult { tool, ok: true, .. } if tool == "ask_user"
        )));
    }

    #[tokio::test]
    async fn max_turns_exhaustion_runs_wrap_up_with_final_text() {
        // 步数耗尽必须补一次不带工具的收尾总结，让回合以可见结论结束。
        let agent = scripted_agent(ScriptedBehavior::ToolCallsThenWrapUp, 2);
        let mut session = Session::new(std::env::temp_dir(), "test-model", None);
        let approver = crate::permissions::AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let mut finals = Vec::new();
        let outcome = agent
            .run_turn(
                &mut session,
                "审查这个仓库",
                &approver,
                &abort,
                &mut |event| {
                    if let TurnEvent::Final { text } = event {
                        finals.push(text.clone());
                    }
                },
            )
            .await
            .expect("收尾总结应让回合正常结束");
        assert_eq!(outcome.final_text.as_deref(), Some("最终结论"));
        assert_eq!(finals, vec!["最终结论".to_string()]);
        assert_eq!(outcome.steps, 2);
        assert_eq!(
            session
                .messages
                .last()
                .and_then(|message| message.content.as_deref()),
            Some("最终结论")
        );
    }
}
