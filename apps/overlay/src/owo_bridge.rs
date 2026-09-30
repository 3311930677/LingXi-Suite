//! owo-agent 引擎桥接（桌宠「重环」）。
//!
//! - 状态探测：`owo_status`（UI 顶栏显示引擎在线/离线）；
//! - 服务托管：`owo_start_service` 拉起 `owo-agent.exe serve`，Job Object
//!   绑定（KILL_ON_JOB_CLOSE）：桌宠退出时引擎不残留；
//! - 会话与回合：`owo_ensure_session` / `owo_send`（后台线程跑 SSE，
//!   逐帧 `owo://turn` 事件推前端，不阻塞 UI）；
//! - 人在回路：`owo_permission` 回传审批；`owo_diff` / `owo_revert` 审阅与回滚。
//!
//! 契约细节见 `crates/owo-bridge` 与 `docs/owo-agent桥接方案.md`。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use owo_bridge::{BridgeError, FileDiff, OwoBridgeClient, SessionInfo, TurnRequest};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::settings::{persist_backend_settings, BackendSettings};
use crate::state::{AppState, MutexExt};

/// SSE 帧推送到前端的事件通道名。
pub(crate) const TURN_EVENT: &str = "owo://turn";

/// 桥接运行态：当前会话 + 托管的引擎子进程。
#[derive(Default)]
pub(crate) struct OwoBridgeState {
    /// 当前活动会话 id（None = 尚未建立，发送前先调用 `owo_ensure_session`）。
    pub(crate) session_id: Option<String>,
    /// 当前会话绑定的工作区。引擎的沙箱根在启动时确定（`serve --workspace`），
    /// 会话 workspace 必须与之一致，否则文件工具报“路径越界”；
    /// 用户改工作区后必须重建会话。
    pub(crate) session_workspace: Option<String>,
    /// 桌宠托管的引擎进程（None = 未托管，可能是用户自己启动的服务）。
    pub(crate) service: Option<OwoService>,
}

/// 桌宠托管的引擎子进程。
///
/// Drop 时先关闭 Job Object（KILL_ON_JOB_CLOSE 立即终止整棵进程树），
/// 再兜底 `kill()`，确保桌宠退出后不留下孤儿引擎进程。
pub(crate) struct OwoService {
    child: Child,
    #[cfg(windows)]
    job: Option<isize>,
}

impl Drop for OwoService {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job_object::close(job);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 前端收到的回合事件负载（`data` 为引擎 tagged JSON，前端自行解析 `type`）。
#[derive(serde::Serialize, Clone)]
pub(crate) struct TurnEventPayload {
    pub(crate) session_id: String,
    pub(crate) event: Option<String>,
    pub(crate) data: String,
    /// 错误分类（仅 `bridge_error` 事件携带）：`unreachable` / `busy_409` /
    /// `unauthorized` / `http` / `stream`。前端按类别渲染文案，避免把
    /// 409（上一轮还在跑）误报成「引擎连接中断」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<&'static str>,
}

/// 引擎状态视图。
#[derive(serde::Serialize)]
pub(crate) struct OwoStatusView {
    pub(crate) reachable: bool,
    pub(crate) version: String,
    pub(crate) auto_approve: bool,
    pub(crate) session_id: Option<String>,
    pub(crate) service_managed: bool,
    pub(crate) base_url: String,
    pub(crate) workspace: String,
    pub(crate) exe_path: Option<String>,
    pub(crate) error: Option<String>,
}

fn client(settings: &BackendSettings) -> OwoBridgeClient {
    OwoBridgeClient::new(settings.owo_agent_base_url())
}

/// 引擎可执行文件路径（候选顺序）：
/// 环境变量 `OWO_AGENT_EXE` > 用户配置（设置页可选）> 应用同目录 / 同目录
/// `resources` 与 `engine` 子目录（安装形态）> 按构建目录推导的仓库构建产物
/// （开发形态；发布机上路径不存在会自然跳过——不再硬编码某台机器的盘符）。
fn resolve_exe_path(settings: &BackendSettings) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(env_path) = std::env::var("OWO_AGENT_EXE") {
        if !env_path.trim().is_empty() {
            candidates.push(PathBuf::from(env_path));
        }
    }
    if !settings.owo_agent_exe_path.trim().is_empty() {
        candidates.push(PathBuf::from(settings.owo_agent_exe_path.trim()));
    }
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            // 安装形态：引擎与主程序同目录，或随 bundle resources / engine 子目录分发。
            candidates.push(dir.join("owo-agent.exe"));
            candidates.push(dir.join("resources").join("owo-agent.exe"));
            candidates.push(dir.join("engine").join("owo-agent.exe"));
        }
    }
    // 开发形态：build 时的 crate 目录 → 兄弟仓库 OwO/agent-sdk/target（debug 优先）。
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent().and_then(Path::parent) {
        if let Some(workspace_root) = repo_root.parent() {
            for profile in ["debug", "release"] {
                candidates.push(
                    workspace_root
                        .join("OwO")
                        .join("agent-sdk")
                        .join("target")
                        .join(profile)
                        .join("owo-agent.exe"),
                );
            }
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// `pick_engine_exe`：打开文件选择器挑选 owo-agent.exe（取消返回 None）。
#[tauri::command]
pub(crate) async fn pick_engine_exe() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("选择 owo-agent.exe")
            .add_filter("可执行文件", &["exe"])
            .pick_file()
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

/// 在阻塞线程池里跑同步 HTTP，避免卡住 Tauri 主线程。
async fn blocking<T: Send + 'static>(task: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| error.to_string())
}

/// 引擎要的是 OpenAI 兼容 base URL（通常以 `/v1` 结尾）。
/// 用户只填域名（如 `https://api.openai.com`、`https://api.deepseek.com`）时补 `/v1`；
/// 已带路径的（`https://host/v1`、本地 mock `http://127.0.0.1:8990/v1`）原样使用。
fn normalize_openai_base(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    if without_scheme.contains('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// 等待引擎端口就绪（最多 ~6 秒）。
fn wait_until_ready(settings: &BackendSettings) -> bool {
    for _ in 0..30 {
        if client(settings).health().is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// `owo_status`：引擎在线状态 + 当前会话 + 托管情况。
#[tauri::command]
pub(crate) async fn owo_status(state: State<'_, AppState>) -> Result<OwoStatusView, String> {
    let (settings, session_id, service_managed) = {
        let settings = state.backend.safe_lock().clone();
        let bridge = state.owo.safe_lock();
        (settings, bridge.session_id.clone(), bridge.service.is_some())
    };
    let probe_settings = settings.clone();
    let probe = blocking(move || {
        let health = client(&probe_settings).health();
        (probe_settings, health)
    })
    .await?;
    let (settings, health) = probe;
    let (reachable, version, auto_approve, error) = match health {
        Ok(health) => (health.healthy, health.version, health.auto_approve, None),
        Err(error) => (false, String::new(), false, Some(error.to_string())),
    };
    Ok(OwoStatusView {
        reachable,
        version,
        auto_approve,
        session_id,
        service_managed,
        base_url: settings.owo_agent_base_url(),
        workspace: settings.owo_agent_workspace_dir().to_string_lossy().to_string(),
        exe_path: resolve_exe_path(&settings).map(|path| path.to_string_lossy().to_string()),
        error,
    })
}

/// 拉起引擎进程并纳入托管（供命令与开机自启复用）。
pub(crate) fn spawn_managed_service(
    state: &AppState,
    settings: &BackendSettings,
) -> Result<(), String> {
    let exe = resolve_exe_path(settings).ok_or_else(|| {
        "未找到 owo-agent.exe：请在设置页填写引擎路径，或设置环境变量 OWO_AGENT_EXE。".to_string()
    })?;
    let workspace = settings.owo_agent_workspace_dir();
    let port = settings.owo_agent_port;
    let mut command = Command::new(&exe);
    command
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workspace")
        .arg(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // 把桌宠的模型设置注入引擎：引擎从 OPENAI_* 环境变量读取模型配置，
    // 这样用户只需在设置页填一次 Endpoint / Model / API Key。
    if !settings.api_key.trim().is_empty() {
        command.env("OPENAI_API_KEY", settings.api_key.trim());
    }
    if !settings.endpoint.trim().is_empty() {
        command.env("OPENAI_BASE_URL", normalize_openai_base(&settings.endpoint));
    }
    if !settings.model.trim().is_empty() {
        command.env("OPENAI_MODEL", settings.model.trim());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：桌宠不应弹出控制台窗口。
        command.creation_flags(0x0800_0000);
    }
    let child = command.spawn().map_err(|error| error.to_string())?;
    #[cfg(windows)]
    let job = job_object::attach(child.id());
    #[cfg(not(windows))]
    let job = ();
    state.owo.safe_lock().service = Some(OwoService { child, job });
    Ok(())
}

/// `owo_start_service`：拉起引擎服务（已在线则直接复用）。
#[tauri::command]
pub(crate) async fn owo_start_service(state: State<'_, AppState>) -> Result<OwoStatusView, String> {
    let settings = state.backend.safe_lock().clone();

    // 托管进程可能已退出：先回收句柄。
    if state.owo.safe_lock().service.is_some() {
        let probe_settings = settings.clone();
        let alive = blocking(move || client(&probe_settings).health().is_ok()).await?;
        if alive {
            return owo_status(state).await;
        }
        state.owo.safe_lock().service = None;
    }

    let probe_settings = settings.clone();
    let reachable = blocking(move || client(&probe_settings).health().is_ok()).await?;
    if reachable {
        // 用户自行启动的服务：不托管，直接使用。
        return owo_status(state).await;
    }

    spawn_managed_service(&state, &settings)?;
    // 等端口就绪（避免 UI 立刻收到"不可达"）。
    let wait_settings = settings.clone();
    let _ = blocking(move || wait_until_ready(&wait_settings)).await;
    owo_status(state).await
}

/// `owo_stop_service`：停止托管进程（用户自行启动的服务不受影响）。
#[tauri::command]
pub(crate) fn owo_stop_service(state: State<AppState>) -> Result<(), String> {
    state.owo.safe_lock().service = None;
    Ok(())
}

/// `owo_restart_service`：重启托管引擎。
///
/// 切换工作文件夹后必须重启：引擎的沙箱根在启动时由 `--workspace` 确定。
/// 重启会作废当前会话（工作区可能已变化），下次发送自动新建。
#[tauri::command]
pub(crate) async fn owo_restart_service(
    state: State<'_, AppState>,
) -> Result<OwoStatusView, String> {
    let settings = state.backend.safe_lock().clone();
    // 释放旧进程（Drop 终止），等端口腾空。
    state.owo.safe_lock().service = None;
    let wait_settings = settings.clone();
    let _ = blocking(move || {
        for _ in 0..30 {
            if client(&wait_settings).health().is_err() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        false
    })
    .await?;
    {
        let mut bridge = state.owo.safe_lock();
        bridge.session_id = None;
        bridge.session_workspace = None;
    }
    spawn_managed_service(&state, &settings)?;
    let wait_settings = settings.clone();
    let _ = blocking(move || wait_until_ready(&wait_settings)).await;
    owo_status(state).await
}

/// `pick_workspace_folder`：打开系统文件夹选择器（取消返回 None）。
#[tauri::command]
pub(crate) async fn pick_workspace_folder() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("选择重任务工作文件夹")
            .pick_folder()
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

/// 引擎设置视图（工作区 / 端口 / 自启 / 模型凭据状态）。
#[derive(serde::Serialize)]
pub(crate) struct EngineOptionsView {
    /// 配置值（空 = 未指定，回落文档目录）。
    pub(crate) workspace: String,
    /// 实际生效的工作文件夹。
    pub(crate) workspace_effective: String,
    pub(crate) port: u16,
    pub(crate) auto_start: bool,
    /// 引擎将复用的模型配置（来自「模型服务」设置）。
    pub(crate) model_endpoint: String,
    pub(crate) model_name: String,
    pub(crate) api_key_configured: bool,
    pub(crate) exe_path: Option<String>,
    /// 用户显式配置的引擎路径（输入框回填用；空 = 未配置，自动探测）。
    pub(crate) exe_configured: String,
}

fn engine_options_view(settings: &BackendSettings) -> EngineOptionsView {
    EngineOptionsView {
        workspace: settings.owo_agent_workspace.clone(),
        workspace_effective: settings
            .owo_agent_workspace_dir()
            .to_string_lossy()
            .to_string(),
        port: settings.owo_agent_port,
        auto_start: settings.owo_agent_auto_start,
        model_endpoint: normalize_openai_base(&settings.endpoint),
        model_name: settings.model.clone(),
        api_key_configured: !settings.api_key.trim().is_empty(),
        exe_path: resolve_exe_path(settings).map(|path| path.to_string_lossy().to_string()),
        exe_configured: settings.owo_agent_exe_path.clone(),
    }
}

/// `get_engine_options`：读取引擎设置（设置页回填）。
#[tauri::command]
pub(crate) fn get_engine_options(state: State<AppState>) -> EngineOptionsView {
    let settings = state.backend.safe_lock();
    engine_options_view(&settings)
}

/// `save_engine_options`：保存工作文件夹 / 端口 / 自启开关 / 引擎路径。
#[tauri::command]
pub(crate) fn save_engine_options(
    state: State<AppState>,
    workspace: String,
    port: u16,
    auto_start: bool,
    exe_path: Option<String>,
) -> Result<EngineOptionsView, String> {
    let workspace = workspace.trim().to_string();
    if !workspace.is_empty() && !std::path::Path::new(&workspace).is_dir() {
        return Err(format!("工作文件夹不存在或不是目录：{workspace}"));
    }
    if port == 0 {
        return Err("端口必须大于 0".into());
    }
    let mut settings = state.backend.safe_lock();
    // B4-2：引擎路径可手选（留空 = 恢复自动探测）；填了就必须是有效的 exe。
    if let Some(exe) = exe_path {
        let exe = exe.trim().to_string();
        if !exe.is_empty() {
            let path = std::path::Path::new(&exe);
            if !path.is_file() {
                return Err(format!("引擎路径不是有效文件：{exe}"));
            }
            if !exe.to_ascii_lowercase().ends_with(".exe") {
                return Err("引擎路径需为 .exe 文件".into());
            }
        }
        settings.owo_agent_exe_path = exe;
    }
    settings.owo_agent_workspace = workspace;
    settings.owo_agent_port = port;
    settings.owo_agent_auto_start = auto_start;
    persist_backend_settings(&settings)?;
    Ok(engine_options_view(&settings))
}

/// `owo_ensure_session`：建立/复用会话，返回会话 id。
///
/// 工作区变更（用户改了设置）时必须重建会话：引擎沙箱根 = 启动时的
/// `--workspace`，旧会话的工作区与新的不一致会让文件工具全部越界失败。
#[tauri::command]
pub(crate) async fn owo_ensure_session(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state.backend.safe_lock().clone();
    let workspace = settings
        .owo_agent_workspace_dir()
        .to_string_lossy()
        .to_string();
    {
        let bridge = state.owo.safe_lock();
        if let Some(existing) = &bridge.session_id {
            if bridge.session_workspace.as_deref() == Some(workspace.as_str()) {
                return Ok(existing.clone());
            }
        }
    }
    let created = blocking({
        let create_settings = settings.clone();
        let create_workspace = workspace.clone();
        move || {
            client(&create_settings)
                .create_session(&create_workspace, None, None)
                .map_err(|error| error.to_string())
        }
    })
    .await?;
    let created = created?;
    {
        let mut bridge = state.owo.safe_lock();
        bridge.session_id = Some(created.id.clone());
        bridge.session_workspace = Some(workspace);
    }
    Ok(created.id)
}

/// `owo_history`：拉取当前活动会话的消息历史（重开对话面板时回放）。
///
/// 无活动会话时返回空消息数组（前端静默）。
#[tauri::command]
pub(crate) async fn owo_history(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    let session_id = state.owo.safe_lock().session_id.clone();
    let Some(session_id) = session_id else {
        return Ok(serde_json::json!({ "messages": [] }));
    };
    blocking(move || {
        client(&settings)
            .session_detail(&session_id)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_sessions`：会话列表（最近更新在前，由服务端排序）。
#[tauri::command]
pub(crate) async fn owo_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || client(&settings).list_sessions().map_err(|e| e.to_string())).await?
}

/// `owo_use_session`：切换当前会话（工作区从服务端详情读取并记录）。
#[tauri::command]
pub(crate) async fn owo_use_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    let lookup_id = session_id.clone();
    let detail = blocking(move || {
        client(&settings)
            .session_detail(&lookup_id)
            .map_err(|error| error.to_string())
    })
    .await?;
    let workspace = detail
        .as_ref()
        .ok()
        .and_then(|value| value.get("workspace"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let mut bridge = state.owo.safe_lock();
    bridge.session_id = Some(session_id);
    bridge.session_workspace = workspace;
    Ok(())
}

/// 由 SSE 帧推导桌宠状态（M1 仲裁输入）。
///
/// - 工具/模型活动 → thinking；出字 → speaking（打字机）；
/// - 审批请求 / 回合失败 / 工具失败 → alert（需人工关注）；
/// - heartbeat 型帧（started 等）不改变状态。
fn pet_state_for(event: &str, data: &str) -> Option<&'static str> {
    match event {
        "progress" | "tool_use" | "model_call" => Some("thinking"),
        "token_delta" | "reasoning_delta" => Some("speaking"),
        "permission_request" | "user_question" | "turn_failed" => Some("alert"),
        "user_answered" => Some("thinking"),
        "final" => Some("speaking"),
        "tool_result" => {
            let ok = serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
                .unwrap_or(true);
            if ok {
                None
            } else {
                Some("alert")
            }
        }
        _ => None,
    }
}

/// `owo_send`：发送一条任务（后台线程流式消费 SSE，事件推 `owo://turn`）。
#[tauri::command]
pub(crate) async fn owo_send(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    message: String,
    attachments: Option<Vec<String>>,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    let request = TurnRequest::new(message).with_attachments(attachments.unwrap_or_default());
    let client = client(&settings);
    let sid = session_id.clone();
    // 立即返回，回合在后台线程里跑完。
    std::thread::spawn(move || {
        // M1：桌宠状态由本线程统一推导并广播（前端不再各自 owoSetPet）。
        let shared = app.state::<AppState>();
        let result = client.turn_stream(&sid, &request, |frame| {
            let payload = TurnEventPayload {
                session_id: sid.clone(),
                event: frame.event.clone(),
                data: frame.data.clone(),
                kind: None,
            };
            let _ = app.emit(TURN_EVENT, payload);
            if let Some(status) =
                pet_state_for(frame.event.as_deref().unwrap_or_default(), &frame.data)
            {
                crate::pet::broadcast_pet_status(&app, &shared, status);
            }
        });
        // 回合结束（正常或异常）统一回落 idle——alert 只由「回合结束」或「审批已响应」清除。
        crate::pet::broadcast_pet_status(&app, &shared, "idle");
        if let Err(error) = result {
            // 错误分类（B1-4）：409 = 同会话并发回合（上一轮还在跑），不是断连；
            // unreachable / unauthorized 需要引导启动或重启引擎。
            let kind = match &error {
                BridgeError::Unreachable(_) => "unreachable",
                BridgeError::Http { status: 409, .. } => "busy_409",
                BridgeError::Http { status: 401, .. } => "unauthorized",
                BridgeError::Http { .. } => "http",
                BridgeError::Decode(_) | BridgeError::Stream(_) => "stream",
            };
            let payload = TurnEventPayload {
                session_id: sid.clone(),
                event: Some("bridge_error".to_string()),
                data: error.to_string(),
                kind: Some(kind),
            };
            let _ = app.emit(TURN_EVENT, payload);
        }
    });
    Ok(())
}

/// `owo_upload_attachment`：把前端贴图/选图（base64）上传到引擎附件目录，
/// 返回登记后的文件名（sanitize 以服务端为准）；发送回合时放进 `attachments`。
#[tauri::command]
pub(crate) async fn owo_upload_attachment(
    state: State<'_, AppState>,
    session_id: String,
    name: String,
    mime: Option<String>,
    data_b64: String,
) -> Result<String, String> {
    let settings = state.backend.safe_lock().clone();
    let fallback_name = name.clone();
    let registered = blocking(move || {
        client(&settings)
            .upload_attachment(&session_id, &name, mime.as_deref(), &data_b64)
            .map_err(|error| error.to_string())
    })
    .await??;
    Ok(registered
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&fallback_name)
        .to_string())
}

/// `owo_permission`：审批回传（允许/拒绝，可选"记住"；E2 后 remember 由引擎真实消费）。
///
/// `remember_scope`（A6-1 三档）：`None` = 不记；`Some("session")` = 本会话内允许；
/// `Some("forever")` = 永久规则（写入引擎 settings.json）。
#[tauri::command]
pub(crate) async fn owo_permission(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    allow: bool,
    remember: Option<bool>,
    remember_scope: Option<String>,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    // 双层 Result：外层 = spawn_blocking 的 JoinError，内层 = 桥接业务错误。
    blocking(move || {
        client(&settings)
            .respond_permission(
                &session_id,
                &request_id,
                allow,
                remember,
                remember_scope.as_deref(),
            )
            .map_err(|error| error.to_string())
    })
    .await??;
    // M1：审批已响应 → 清除 alert 并回到 thinking（回合继续）。
    crate::pet::clear_pet_alert(&app, &state);
    Ok(())
}

/// `owo_activity`（A8-2）：引擎活跃回合快照（raw JSON 透传）。桌宠进度控制台
/// 每 2.5 秒轮询它，显示工作台/引擎侧正在跑什么、有无待审批。
#[tauri::command]
pub(crate) async fn owo_activity(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || client(&settings).activity().map_err(|error| error.to_string())).await?
}

/// `owo_pet_sync`（A8-3）：桌宠显隐双向同步——心跳上报本机实际状态，
/// 取回工作台开关写入的期望值；不一致（用户在工作台/设置里切换过）时立即切换。
/// 由桌宠轮询循环每 2.5 秒调用一次（与 `/activity` 同节奏）。
#[tauri::command]
pub(crate) async fn owo_pet_sync(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let (settings, actual) = {
        let settings = state.backend.safe_lock().clone();
        let actual = settings.pet_visible;
        (settings, actual)
    };
    let reply = blocking(move || {
        client(&settings)
            .report_pet_visible(actual)
            .map_err(|error| error.to_string())
    })
    .await??;
    let desired = reply.get("desired").and_then(serde_json::Value::as_bool);
    match desired {
        Some(desired) if desired != actual => {
            crate::pet::set_pet_visible(app, state, desired)?;
            Ok(desired)
        }
        Some(desired) => Ok(desired),
        None => Ok(actual),
    }
}

/// A8-3：桌宠显隐同步工作线程——每 5 秒心跳上报本机实际状态，并把工作台写入的
/// 期望值应用到窗口。`owo_pet_sync` 命令的常驻版：不依赖前端定时器存活
/// （桌宠窗口隐藏后 WebView2 会节流其定时器，纯前端轮询在隐藏态不可靠）。
pub(crate) fn spawn_pet_sync_worker(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let state = app.state::<AppState>();
        let settings = state.backend.safe_lock().clone();
        let actual = settings.pet_visible;
        let reply = client(&settings).report_pet_visible(actual).ok();
        let desired = reply
            .as_ref()
            .and_then(|value| value.get("desired"))
            .and_then(serde_json::Value::as_bool);
        if let Some(desired) = desired {
            if desired != actual {
                let _ = crate::pet::set_pet_visible(app.clone(), state, desired);
            }
        }
    });
}

/// `open_workbench`：在系统默认浏览器打开 OwO Agent 工作台（右键菜单/托盘入口）。
#[tauri::command]
pub(crate) fn open_workbench(state: State<AppState>) -> Result<(), String> {
    let port = state.backend.safe_lock().owo_agent_port;
    let url = format!("http://127.0.0.1:{port}/");
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|error| format!("打开工作台失败：{error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("open_workbench 仅支持 Windows".to_string())
    }
}

/// `owo_automations`（A8-1）：定时任务列表（raw JSON 透传）。
#[tauri::command]
pub(crate) async fn owo_automations(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .list_automations()
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_automation_create`：创建定时任务（schedule/action 为引擎协议 JSON）。
#[tauri::command]
pub(crate) async fn owo_automation_create(
    state: State<'_, AppState>,
    name: String,
    schedule: serde_json::Value,
    action: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .create_automation(&name, schedule, action)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_automation_toggle`：启停切换。
#[tauri::command]
pub(crate) async fn owo_automation_toggle(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .toggle_automation(&id)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_automation_delete`：删除任务。
#[tauri::command]
pub(crate) async fn owo_automation_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .delete_automation(&id)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_automation_runs`：执行记录（task_id/limit 可选，时间倒序）。
#[tauri::command]
pub(crate) async fn owo_automation_runs(
    state: State<'_, AppState>,
    task_id: Option<String>,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .automation_runs(task_id.as_deref(), limit)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_permission_rules`（B4-1）：权限规则列表（持久 + 会话临时，raw JSON 透传）。
#[tauri::command]
pub(crate) async fn owo_permission_rules(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .permission_rules()
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_remove_permission_rule`（B4-1）：删除一条权限规则。
#[tauri::command]
pub(crate) async fn owo_remove_permission_rule(
    state: State<'_, AppState>,
    tool: String,
    pattern: String,
    session: bool,
) -> Result<bool, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .remove_permission_rule(&tool, &pattern, session)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// `owo_answer`：回答引擎的 ask_user 提问（唤醒挂起中的回合）。
#[tauri::command]
pub(crate) async fn owo_answer(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    question_id: String,
    answer: String,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || {
        client(&settings)
            .answer_question(&session_id, &question_id, &answer)
            .map_err(|error| error.to_string())
    })
    .await??;
    // M1：已回答 → 清除 alert（回合继续）。
    crate::pet::clear_pet_alert(&app, &state);
    Ok(())
}

/// `owo_diff`：当前会话的改动列表。
#[tauri::command]
pub(crate) async fn owo_diff(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<FileDiff>, String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || client(&settings).diff(&session_id).map_err(|e| e.to_string())).await?
}

/// `owo_revert`：回滚当前会话的全部改动。
#[tauri::command]
pub(crate) async fn owo_revert(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || client(&settings).revert(&session_id).map_err(|e| e.to_string())).await?
}

/// `owo_abort`：中止当前回合。
#[tauri::command]
pub(crate) async fn owo_abort(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    blocking(move || client(&settings).abort(&session_id).map_err(|e| e.to_string())).await?
}

/// `owo_abort_current`（M2-lite）：桌宠双击急停——中止当前活动会话的回合。
///
/// 前端无需知道会话 id；无活动会话时静默成功。
#[tauri::command]
pub(crate) async fn owo_abort_current(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.backend.safe_lock().clone();
    let session_id = state.owo.safe_lock().session_id.clone();
    let Some(session_id) = session_id else {
        return Ok(());
    };
    blocking(move || {
        client(&settings)
            .abort(&session_id)
            .map_err(|error| error.to_string())
    })
    .await??;
    // 立即回落 idle：结束后 Rust 侧也会收到流关闭再广播一次（幂等）。
    crate::pet::broadcast_pet_status(&app, &state, "idle");
    Ok(())
}

/// Job Object 托管：KILL_ON_JOB_CLOSE，句柄关闭即终止引擎及其子进程。
#[cfg(windows)]
mod job_object {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    /// 创建 Job 并绑定进程；失败返回 None（降级为 Drop 时 kill 子进程）。
    pub(super) fn attach(pid: u32) -> Option<isize> {
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_err()
            {
                let _ = CloseHandle(job);
                return None;
            }
            let process = match OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
                Ok(process) => process,
                Err(_) => {
                    let _ = CloseHandle(job);
                    return None;
                }
            };
            let assigned = AssignProcessToJobObject(job, process);
            let _ = CloseHandle(process);
            if assigned.is_err() {
                let _ = CloseHandle(job);
                return None;
            }
            Some(job.0 as isize)
        }
    }

    pub(super) fn close(handle: isize) {
        unsafe {
            let _ = CloseHandle(HANDLE(handle as *mut core::ffi::c_void));
        }
    }
}
