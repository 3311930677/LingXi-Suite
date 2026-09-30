//! 桌宠与引擎桥接配置：加载/落盘（`%APPDATA%/lingxi/settings.json`）。
//!
//! 主面板退役后，本模块只承载桌宠（皮肤/气泡/可见性）与引擎桥接
//! （exe 路径/端口/工作区/自启）两类配置；模型凭据等由引擎侧
//! `settings.json`（工作台设置页）统一管理，不再经桌宠中转。

use serde::{Deserialize, Serialize};

/// 持久化配置。旧版本文件中的退役字段（backend/endpoint/api_key/panel_* 等）
/// 经 `serde(default)` + 忽略未知字段的方式平滑遗忘。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BackendSettings {
    /// 当前桌宠皮肤 id（见 ui/assets/skins/）。
    pub(crate) pet_skin: String,
    /// 用户自定义气泡文案（None = 使用皮肤默认文案）。
    pub(crate) pet_bubble_overrides: crate::pet_skin::PetBubbleOverrides,
    /// 桌宠窗口是否显示。
    pub(crate) pet_visible: bool,
    /// owo-agent 引擎可执行文件路径（留空 = 用内置默认候选路径探测）。
    pub(crate) owo_agent_exe_path: String,
    /// owo-agent 引擎服务端口（默认 4096，与 start-agent-server.cmd 一致）。
    pub(crate) owo_agent_port: u16,
    /// 重任务默认工作区（留空 = 用户文档目录）。
    pub(crate) owo_agent_workspace: String,
    /// 桌宠启动时自动拉起引擎服务。
    pub(crate) owo_agent_auto_start: bool,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            pet_skin: crate::pet_skin::DEFAULT_SKIN_ID.to_string(),
            pet_bubble_overrides: Default::default(),
            pet_visible: true,
            owo_agent_exe_path: String::new(),
            owo_agent_port: owo_bridge::DEFAULT_PORT,
            owo_agent_workspace: String::new(),
            owo_agent_auto_start: true,
        }
    }
}

impl BackendSettings {
    /// 引擎服务地址（本机回环 + 配置端口）。
    pub(crate) fn owo_agent_base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.owo_agent_port)
    }

    /// 重任务工作区：配置为空时回落到用户文档目录。
    pub(crate) fn owo_agent_workspace_dir(&self) -> std::path::PathBuf {
        let configured = self.owo_agent_workspace.trim();
        if !configured.is_empty() {
            return std::path::PathBuf::from(configured);
        }
        dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
}

pub(crate) fn backend_settings_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("settings.json"))
}

pub(crate) fn load_backend_settings() -> BackendSettings {
    backend_settings_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<BackendSettings>(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn persist_backend_settings(settings: &BackendSettings) -> Result<(), String> {
    let path = backend_settings_path().ok_or("cannot resolve config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}
