//! 窗口位置持久化：主面板与桌宠的落盘/恢复。
//!
//! 与 settings.json 分开存放：位置数据高频变化（拖动时每 500ms 防抖写一次），
//! 行为开关仍在 settings.json 里，避免读写竞争。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Physical pixels，与 Win32 / Tauri 的 PhysicalPosition 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPos {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// 用户拖动后的主面板位置（None = 从未拖动，保持跟随光标）。
    pub panel: Option<WindowPos>,
    /// 用户拖动后的桌宠位置（None = 使用默认右下角）。
    pub pet: Option<WindowPos>,
}

fn window_state_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lingxi").join("window-state.json"))
}

pub fn load() -> WindowState {
    window_state_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<WindowState>(&bytes).ok())
        .unwrap_or_default()
}

pub fn persist(state: &WindowState) -> Result<(), String> {
    let path = window_state_path().ok_or("cannot resolve config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let json = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}
