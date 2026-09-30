//! 进程内共享状态：`AppState` 与互斥锁防中毒扩展。
//!
//! 各命令模块通过 `State<'_, AppState>` 访问这里定义的字段；
//! 字段全部 `pub(crate)`，仅本 crate 可见。

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;

use assistant_core::{SelectionSnapshot, WriteReceipt};
use lingxi_agent::Session;
use lingxi_tools::ToolRegistry;
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

/// A preview result cached so "apply" can reuse it instead of running the model
/// a second time. Storing the exact `(mode, source)` it was computed for lets
/// `apply_transform` verify the preview still matches the captured selection
/// before writing it back.
#[derive(Clone)]
pub(crate) struct CachedPreview {
    pub(crate) mode: String,
    pub(crate) source: String,
    pub(crate) transformed: String,
    pub(crate) backend_signature: String,
}

/// Shared state for rewrite, pet and semi-automatic chat flows.
pub(crate) struct AppState {
    pub(crate) snapshot: Mutex<Option<SelectionSnapshot>>,
    pub(crate) last_receipt: Mutex<Option<WriteReceipt>>,
    /// The most recent preview, reused by `apply_transform` so it doesn't re-run
    /// inference (which is slow and, being sampled, could differ from what the
    /// user just saw — and whose latency widens the focus-drift window).
    pub(crate) last_preview: Mutex<Option<CachedPreview>>,
    pub(crate) backend: Mutex<crate::settings::BackendSettings>,
    pub(crate) pet_status: Mutex<String>,
    #[allow(dead_code)]
    pub(crate) last_qq_message: Mutex<Option<String>>,
    /// Incremented for every successful hotkey capture, even when the selected
    /// text is identical to the previous capture.
    pub(crate) selection_revision: AtomicU64,
    /// Once the user drags the panel, preserve that preferred position instead
    /// of snapping it back beside the cursor on every invocation.
    pub(crate) user_positioned: AtomicBool,
    /// 窗口位置持久化数据（panel/pet），启动时从磁盘加载。
    pub(crate) window_state: Mutex<window_state::WindowState>,
    /// 防抖写盘调度标志：拖动期间高频 Moved 事件只触发一次落盘任务。
    pub(crate) window_save_pending: AtomicBool,
    /// 程序化 set_position 的目标位置：与 Moved 事件比对，区分"程序摆放"
    /// 与"用户拖动"（只有用户拖动才写入持久化）。
    pub(crate) last_programmatic_pos:
        Mutex<std::collections::HashMap<String, PhysicalPosition<i32>>>,
    /// Agent tool registry, initialized with all Windows tools at startup.
    pub(crate) tool_registry: std::sync::Mutex<ToolRegistry>,
    /// Agent conversation session, persists across messages within a session.
    pub(crate) agent_session: std::sync::Mutex<Session>,
    /// 已注册进 tool_registry 的插件工具：插件 id → 工具名。
    /// 卸载/同步时按值反向移除注册表项；list_tools 据此标注 source。
    pub(crate) plugin_tool_map: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// 取色放大镜：进入取色时的全屏截图 data URL，供 color-lens 窗口拉取。
    pub(crate) lens_image: Mutex<Option<String>>,
    /// owo-agent 引擎桥接（当前会话 + 托管子进程）。
    pub(crate) owo: Mutex<crate::owo_bridge::OwoBridgeState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(None),
            last_receipt: Mutex::new(None),
            last_preview: Mutex::new(None),
            backend: Mutex::new(crate::settings::load_backend_settings()),
            pet_status: Mutex::new("idle".into()),
            last_qq_message: Mutex::new(None),
            selection_revision: AtomicU64::new(0),
            user_positioned: AtomicBool::new(false),
            window_state: Mutex::new(window_state::load()),
            window_save_pending: AtomicBool::new(false),
            last_programmatic_pos: Mutex::new(Default::default()),
            tool_registry: std::sync::Mutex::new({
                let mut reg = ToolRegistry::new();
                lingxi_tools_windows::register_default_tools(&mut reg);
                reg
            }),
            agent_session: std::sync::Mutex::new(crate::agent::load_agent_session()),
            plugin_tool_map: std::sync::Mutex::new(Default::default()),
            lens_image: Mutex::new(None),
            owo: Mutex::new(crate::owo_bridge::OwoBridgeState::default()),
        }
    }
}
