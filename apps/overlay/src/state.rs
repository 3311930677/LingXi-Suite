//! 进程内共享状态：`AppState` 与互斥锁防中毒扩展。
//!
//! 各命令模块通过 `State<'_, AppState>` 访问这里定义的字段；
//! 字段全部 `pub(crate)`，仅本 crate 可见。

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tauri::PhysicalPosition;

use crate::window_state;

/// Extension trait so every `Mutex::lock().unwrap()` in the app recovers from
/// poisoning instead of panicking. If one thread panics while holding a lock,
/// the Mutex becomes "poisoned" and subsequent `.safe_lock()` calls would
/// cascade-panic — making the entire overlay unusable after a single error.
/// Recovering the inner data keeps the app running with the last known state.
pub(crate) trait MutexExt<T> {
    fn safe_lock(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn safe_lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Shared state of the pet-only overlay.
pub(crate) struct AppState {
    /// 桌宠/引擎桥接配置（皮肤、可见性、引擎端口与自启）。
    pub(crate) backend: Mutex<crate::settings::BackendSettings>,
    /// 桌宠状态（Rust 侧统一仲裁：idle/thinking/speaking/alert）。
    pub(crate) pet_status: Mutex<String>,
    /// 窗口位置持久化数据（pet），启动时从磁盘加载。
    pub(crate) window_state: Mutex<window_state::WindowState>,
    /// 防抖写盘调度标志：拖动期间高频 Moved 事件只触发一次落盘任务。
    pub(crate) window_save_pending: AtomicBool,
    /// 程序化 set_position 的目标位置：与 Moved 事件比对，区分"程序摆放"
    /// 与"用户拖动"（只有用户拖动才写入持久化）。
    pub(crate) last_programmatic_pos:
        Mutex<std::collections::HashMap<String, PhysicalPosition<i32>>>,
    /// owo-agent 引擎桥接（当前会话 + 托管子进程）。
    pub(crate) owo: Mutex<crate::owo_bridge::OwoBridgeState>,
    /// 桌宠窗口当前是否已导航到引擎侧页面（`/pet/index.html`）。
    /// false = 显示本地 boot 引导页（引擎离线/启动中）。由 pet-sync worker 仲裁。
    pub(crate) pet_remote_loaded: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            backend: Mutex::new(crate::settings::load_backend_settings()),
            pet_status: Mutex::new("idle".into()),
            window_state: Mutex::new(window_state::load()),
            window_save_pending: AtomicBool::new(false),
            last_programmatic_pos: Mutex::new(Default::default()),
            owo: Mutex::new(crate::owo_bridge::OwoBridgeState::default()),
            pet_remote_loaded: AtomicBool::new(false),
        }
    }
}
