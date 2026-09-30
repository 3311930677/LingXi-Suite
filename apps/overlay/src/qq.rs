//! QQ 集成：会话探测、选区读取、草稿生成与写入。
//!
//! generate_qq_draft 复用 rewrite::run_model。

use assistant_inference::ModelTask;
use assistant_windows::{
    capture_qq_selection_text, qq_write_draft, remember_foreground_if_qq, resolve_qq_window,
};
use serde::Serialize;
use std::thread;
use std::time::Duration;
use tauri::State;

use crate::state::{AppState, MutexExt};

#[derive(Serialize)]
pub(crate) struct QqMessageView {
    conversation: String,
    message: String,
    is_new: bool,
}

#[derive(Serialize)]
pub(crate) struct DraftWriteResult {
    verified: bool,
}

/// Check whether QQ is the foreground window, and if so return the active
/// conversation title (the chat partner's display name). The caller (UI) then
/// asks the user to select the message they want to reply to and calls
/// `capture_qq_selection` to read it via the standard Win32 selection API.
///
/// This intentionally does NOT scan QQ's UIA tree: QQNT (Chromium / Electron)
/// does not expose sender identity per bubble, so any "auto-read latest
/// message" attempt ends up mixing own messages, the contact list preview,
/// and group member rosters. Reading the user's explicit selection is the
/// only reliable source of truth for "which message to reply to".
#[tauri::command]
pub(crate) async fn qq_poll_latest(
    _state: State<'_, AppState>,
) -> Result<Option<QqMessageView>, String> {
    let info = tauri::async_runtime::spawn_blocking(resolve_qq_window)
        .await
        .map_err(|e| format!("foreground probe failed: {e}"))?;
    // `resolve_qq_window` already falls back to the last remembered QQ window
    // (recorded by the background sampler every 500ms) when the live foreground
    // is something else — typically this LingXi panel after the user clicked a
    // button. Only when neither is available do we tell the UI QQ isn't around.
    match info {
        Ok(info) => Ok(Some(QqMessageView {
            conversation: info.title,
            message: String::new(),
            is_new: false,
        })),
        Err(_) => Ok(None),
    }
}

/// Capture the user's current text selection inside QQ. The user must select
/// the message they want to reply to (double-click or drag-select) before
/// calling this. Returns the selected text verbatim.
///
/// Unlike `capture_selection` (which needs QQ in the foreground), this scans
/// the remembered QQ window's UIA tree for a TextPattern element with a
/// non-empty selection — so the DOM selection survives even after the panel
/// steals keyboard focus.
#[tauri::command]
pub(crate) async fn capture_qq_selection(
    _state: State<'_, AppState>,
) -> Result<QqMessageView, String> {
    let (info, message) = tauri::async_runtime::spawn_blocking(|| {
        let info = resolve_qq_window();
        let text = capture_qq_selection_text();
        (info, text)
    })
    .await
    .map_err(|e| format!("selection capture failed: {e}"))?;

    let message = message.map_err(|e| format!("selection capture: {e}"))?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("没有选中文字。请在 QQ 里双击或拖选对方的消息后再试。".into());
    }
    let conversation = match info {
        Ok(i) => i.title,
        Err(_) => String::new(),
    };
    Ok(QqMessageView {
        conversation,
        message,
        is_new: true,
    })
}

#[tauri::command]
pub(crate) async fn generate_qq_draft(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message: String,
) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("没有可回复的 QQ 消息".into());
    }
    let settings = state.backend.safe_lock().clone();
    // B6-3：统一走广播（此前直写 mutex，桌宠只认事件 → 无反应）。
    crate::pet::broadcast_pet_status(&app, &state, "thinking");
    // Flatten JoinError and model error into one Result so the match below
    // resets pet_status on every failure path (including task panic).
    let draft = tauri::async_runtime::spawn_blocking(move || {
        crate::rewrite::run_model(&settings, ModelTask::ChatReply, &message)
    })
    .await
    .map_err(|error| format!("draft task failed: {error}"))
    .and_then(|inner| inner);
    match draft {
        Ok(draft) => {
            crate::pet::broadcast_pet_status(&app, &state, "speaking");
            Ok(draft)
        }
        Err(error) => {
            crate::pet::broadcast_pet_status(&app, &state, "idle");
            Err(error)
        }
    }
}

/// Write the user-confirmed draft into QQ's composer. There is deliberately no
/// send command: the user reviews the text and presses Send inside QQ.
#[tauri::command]
pub(crate) async fn write_qq_draft(draft: String) -> Result<DraftWriteResult, String> {
    let verified = tauri::async_runtime::spawn_blocking(move || qq_write_draft(&draft))
        .await
        .map_err(|error| format!("QQ write task failed: {error}"))?
        .map_err(|error| error.to_string())?;
    Ok(DraftWriteResult { verified })
}

/// Continuously remember the last foreground QQ window so the panel's "read"
/// and "write draft" buttons still target QQ after the panel takes focus. This
/// is a cheap Win32-only poll (no COM/UIA), safe to run at a modest cadence.
pub(crate) fn spawn_qq_foreground_sampler() {
    thread::spawn(|| loop {
        remember_foreground_if_qq();
        thread::sleep(Duration::from_millis(500));
    });
}
