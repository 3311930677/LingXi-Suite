//! Agent 命令：云端对话、历史与重置、工具注册表查询与开关。
//!
//! 会话持久化到 `agent-session.json`（写入走临时文件 + 原子改名）；
//! 危险工具在安全模式下整体拒绝（DenyAll），待逐次审批流程接入后放开。

use lingxi_agent::{AgentBackend, AgentEngine, AgentRunReport, Session};
use lingxi_tools::{ConfirmGate, DenyAll, RiskLevel, ToolRegistry};
use serde::Serialize;
use tauri::State;

use crate::state::{AppState, MutexExt};
use assistant_inference::{CloudAgentBackend, CloudConfig};

fn default_agent_working_dir() -> std::path::PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
}

fn agent_session_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("agent-session.json"))
}

pub(crate) fn load_agent_session() -> Session {
    let mut session = agent_session_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Session>(&bytes).ok())
        .unwrap_or_else(|| {
            Session::new(
                uuid::Uuid::new_v4().to_string(),
                default_agent_working_dir(),
            )
        });
    if !session.working_dir.is_dir() {
        session.working_dir = default_agent_working_dir();
    }
    session.trim_history(40);
    session
}

fn persist_agent_session(session: &Session) -> Result<(), String> {
    let path = agent_session_path().ok_or("无法解析灵犀配置目录")?;
    let parent = path.parent().ok_or("无法解析会话目录")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let json = serde_json::to_vec_pretty(session).map_err(|error| error.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, json).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// A tool's metadata for the frontend.
#[derive(Serialize)]
pub(crate) struct ToolView {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) risk_level: String,
    pub(crate) enabled: bool,
    /// "builtin"（内置工具）| "plugin"（市场安装的插件工具）。
    pub(crate) source: String,
}

/// Send a message to the agent and get a reply. The agent may call tools
/// internally before replying. Requires a configured cloud backend.
#[tauri::command]
pub(crate) async fn agent_chat(
    state: State<'_, AppState>,
    message: String,
) -> Result<AgentRunReport, String> {
    let settings = state.backend.safe_lock().clone();
    if settings.backend != "cloud" {
        return Err("Agent 对话暂需云端模型，请在模型设置中切换到 OpenAI 兼容云端。".into());
    }
    if settings.endpoint.is_empty() || settings.api_key.is_empty() {
        return Err("请先在设置页配置云端模型的 Endpoint 和 API Key。".into());
    }

    let config = CloudConfig {
        endpoint: settings.endpoint.clone(),
        model: settings.model.clone(),
        api_key: settings.api_key.clone(),
    };
    let backend = CloudAgentBackend::new(config);
    let backend_box: Box<dyn AgentBackend> = Box::new(backend);

    // Build a fresh registry for this run — tools are stateless, so this is
    // cheap. We copy the user's enabled/disabled state from the stored registry.
    let mut registry = ToolRegistry::new();
    lingxi_tools_windows::register_default_tools(&mut registry);
    // 工具插件与内置工具同权参与本轮对话（enabled 状态由下方循环统一复制）。
    if let Some(root) = crate::market::plugins_root() {
        for plugin in lingxi_tools::plugin::scan_plugins(&root) {
            registry.register(plugin);
        }
    }
    {
        let reg = state.tool_registry.safe_lock();
        for schema in reg.all_schemas() {
            if !reg.is_enabled(&schema.name) {
                registry.set_enabled(&schema.name, false);
            }
        }
    }

    let engine = AgentEngine::new(backend_box, std::sync::Arc::new(registry));
    // Dangerous tools are denied until the overlay gains a per-invocation
    // approval flow. They are also disabled in the default registry, providing
    // defense in depth against accidental shell/file mutations.
    let confirm = std::sync::Arc::new(DenyAll) as std::sync::Arc<dyn ConfirmGate>;

    // Clone the session so we never lose the original if the future is
    // cancelled or panics. A cheap deep copy is far safer than `mem::take`,
    // which would replace the Mutex contents with `Session::default()` and
    // lose all user history if the awaited future is dropped.
    let mut session = state.agent_session.safe_lock().clone();
    let result = engine.run_with_trace(&message, &mut session, confirm).await;
    // Persist and restore the session regardless of model success so the user
    // does not lose prior turns after a transient network error.
    let persist_result = persist_agent_session(&session);
    // Only write back on success to avoid clobbering good history with a
    // partial/corrupted session from a failed run.
    if result.is_ok() {
        *state.agent_session.safe_lock() = session;
    }

    let report = result.map_err(|e| e.to_string())?;
    if let Err(e) = persist_result {
        eprintln!("[lingxi] warning: failed to persist agent session: {e}");
    }
    Ok(report)
}

#[derive(Serialize)]
pub(crate) struct AgentHistoryItem {
    pub(crate) role: String,
    pub(crate) content: String,
}

/// Return user-visible messages from the persisted Agent conversation.
#[tauri::command]
pub(crate) fn agent_history(state: State<AppState>) -> Vec<AgentHistoryItem> {
    use lingxi_agent::Role;

    state
        .agent_session
        .lock()
        .unwrap()
        .messages
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant if message.tool_calls.is_empty() => "assistant",
                _ => return None,
            };
            (!message.content.trim().is_empty()).then(|| AgentHistoryItem {
                role: role.to_string(),
                content: message.content.clone(),
            })
        })
        .collect()
}

/// Reset the agent conversation session (start a new chat).
#[tauri::command]
pub(crate) fn agent_reset(state: State<AppState>) -> Result<(), String> {
    let session = Session::new(
        uuid::Uuid::new_v4().to_string(),
        default_agent_working_dir(),
    );
    persist_agent_session(&session)?;
    *state.agent_session.safe_lock() = session;
    Ok(())
}

/// List all registered tools with their metadata.
#[tauri::command]
pub(crate) fn list_tools(state: State<AppState>) -> Vec<ToolView> {
    let reg = state.tool_registry.safe_lock();
    let plugin_names: Vec<String> = state
        .plugin_tool_map
        .safe_lock()
        .values()
        .cloned()
        .collect();
    reg.all_schemas()
        .iter()
        .map(|s| {
            let risk = reg.risk_of(&s.name).unwrap_or(RiskLevel::Safe);
            ToolView {
                name: s.name.clone(),
                description: s.description.clone(),
                risk_level: format!("{:?}", risk).to_lowercase(),
                enabled: reg.is_enabled(&s.name),
                source: if plugin_names.contains(&s.name) {
                    "plugin".into()
                } else {
                    "builtin".into()
                },
            }
        })
        .collect()
}

/// Enable or disable a tool by name.
#[tauri::command]
pub(crate) fn toggle_tool(
    state: State<AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let mut reg = state.tool_registry.safe_lock();
    let risk = reg
        .risk_of(&name)
        .ok_or_else(|| format!("未知工具: {name}"))?;
    if enabled && risk == RiskLevel::Dangerous {
        return Err("危险工具需要逐次确认，当前安全模式下不可启用。".into());
    }
    reg.set_enabled(&name, enabled);
    Ok(())
}
