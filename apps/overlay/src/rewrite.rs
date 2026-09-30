//! 文本改写管线：模型任务分发、diff 预览、回写与撤销。
//!
//! 对应前端命令 current_selection / preview_transform / apply_transform / undo_last；
//! 热键触发路径 on_transform / on_undo 也在这里。

use assistant_core::{diff_chars, transformer_by_name, DiffOp, InputAdapter};
use assistant_inference::{CloudBackend, CloudConfig, LocalBackend, ModelBackend, ModelTask};
use assistant_windows::WindowsAdapter;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

use crate::placement::position_overlay;
use crate::settings::BackendSettings;
use crate::state::{AppState, CachedPreview, MutexExt};

/// One diff segment, serialized for the frontend.
#[derive(Serialize)]
struct DiffSegment {
    kind: &'static str,
    text: String,
}

#[derive(Serialize)]
pub(crate) struct PreviewResult {
    transformed: String,
    diff: Vec<DiffSegment>,
    /// Non-blocking quality advice. Semantic drift, truncation and runaway
    /// output remain hard errors; a merely conservative rewrite is shown.
    warning: Option<String>,
    /// True only when a local model task is waiting for model preparation.
    pending: bool,
}

fn model_task(mode: &str) -> Option<ModelTask> {
    match mode {
        "polish" => Some(ModelTask::Polish),
        "proofread" => Some(ModelTask::Proofread),
        "prompt-enhance" => Some(ModelTask::PromptEnhance),
        _ => None,
    }
}

fn backend_signature(settings: &BackendSettings) -> String {
    format!(
        "{}|{}|{}",
        settings.backend, settings.endpoint, settings.model
    )
}

pub(crate) fn run_model(
    settings: &BackendSettings,
    task: ModelTask,
    input: &str,
) -> Result<String, String> {
    match settings.backend.as_str() {
        "local" => LocalBackend
            .complete(task, input)
            .map_err(|error| format!("本地模型调用失败: {error:#}")),
        "cloud" => CloudBackend::new(CloudConfig {
            endpoint: settings.endpoint.clone(),
            model: settings.model.clone(),
            api_key: settings.api_key.clone(),
        })
        .complete(task, input)
        .map_err(|error| format!("云端模型调用失败: {error:#}")),
        other => Err(format!("unknown backend: {other}")),
    }
}

fn transform_text(settings: &BackendSettings, mode: &str, input: &str) -> Result<String, String> {
    if let Some(task) = model_task(mode) {
        return run_model(settings, task, input);
    }
    transformer_by_name(mode)
        .map(|transformer| transformer.transform(input))
        .ok_or_else(|| format!("unknown mode: {mode}"))
}

fn to_segments(ops: Vec<DiffOp>) -> Vec<DiffSegment> {
    ops.into_iter()
        .map(|op| match op {
            DiffOp::Equal(text) => DiffSegment {
                kind: "equal",
                text,
            },
            DiffOp::Insert(text) => DiffSegment { kind: "ins", text },
            DiffOp::Delete(text) => DiffSegment { kind: "del", text },
        })
        .collect()
}
#[derive(Serialize)]
pub(crate) struct CurrentSelection {
    text: String,
    revision: u64,
}

/// Text captured at the last hotkey press plus a monotonic capture revision.
/// The revision changes even when the user selects the same sentence again.
#[tauri::command]
pub(crate) fn current_selection(state: State<AppState>) -> CurrentSelection {
    CurrentSelection {
        text: state
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .map(|snapshot| snapshot.selected_text.clone())
            .unwrap_or_default(),
        revision: state.selection_revision.load(Ordering::Acquire),
    }
}

/// Preview a transformation and its diff without writing anything.
///
/// The model runs on a blocking thread pool (`spawn_blocking`) rather than the
/// Tauri command thread: local CPU decoding takes seconds, and doing it inline
/// froze the whole overlay UI. The winning preview is cached so `apply`
/// can reuse it instead of decoding a second time.
#[tauri::command]
pub(crate) async fn preview_transform(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
    text: String,
) -> Result<PreviewResult, String> {
    let settings = state.backend.safe_lock().clone();
    let is_model_task = model_task(&mode).is_some();
    let pending = is_model_task && settings.backend == "local" && !assistant_inference::is_ready();
    if pending {
        return Ok(PreviewResult {
            transformed: text.clone(),
            diff: to_segments(diff_chars(&text, &text)),
            warning: None,
            pending: true,
        });
    }

    // B6-3：统一走广播（此前直写 mutex，桌宠只认事件 → 无反应）。
    crate::pet::broadcast_pet_status(&app, &state, "thinking");
    let transformed = {
        let mode = mode.clone();
        let text = text.clone();
        tauri::async_runtime::spawn_blocking(move || transform_text(&settings, &mode, &text))
            .await
            .map_err(|error| format!("preview task failed: {error}"))
            .and_then(|inner| inner)
    };
    let transformed = match transformed {
        Ok(t) => {
            crate::pet::broadcast_pet_status(&app, &state, "speaking");
            t
        }
        Err(e) => {
            // Reset pet status on any error (task panic or model failure) so
            // the pet is not stuck in "thinking" forever after a transient error.
            crate::pet::broadcast_pet_status(&app, &state, "idle");
            return Err(e);
        }
    };

    let warning = model_task(&mode)
        .and_then(|task| assistant_inference::quality_warning(task, &text, &transformed));
    let diff = to_segments(diff_chars(&text, &transformed));
    let settings = state.backend.safe_lock().clone();
    // Cache this result so a subsequent `apply` for the same mode/source/backend
    // writes back exactly what the user saw, without re-running the model.
    *state.last_preview.safe_lock() = Some(CachedPreview {
        mode,
        source: text,
        backend_signature: backend_signature(&settings),
        transformed: transformed.clone(),
    });
    Ok(PreviewResult {
        transformed,
        diff,
        warning,
        pending: false,
    })
}

/// Apply the transformation to the captured selection and write it back.
///
/// This reuses the cached preview whenever it matches the current selection, so
/// it does NOT run the model again: a second inference would add seconds of
/// latency (widening the focus-drift window that rejects the write) and, being
/// temperature-sampled, could even differ from what the user just approved. It
/// only falls back to a fresh (blocking) inference if no matching preview
/// exists.
#[tauri::command]
pub(crate) async fn apply_transform(
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    let snapshot = state
        .snapshot
        .lock()
        .unwrap()
        .clone()
        .ok_or("no captured selection")?;

    // Fast path: reuse the preview the user actually saw, if it was computed for
    // this exact mode, selection text and model backend configuration.
    let settings = state.backend.safe_lock().clone();
    let signature = backend_signature(&settings);
    let cached = state.last_preview.safe_lock().clone();
    let new_text = match cached {
        Some(preview)
            if preview.mode == mode
                && preview.source == snapshot.selected_text
                && preview.backend_signature == signature =>
        {
            preview.transformed
        }
        _ => {
            let source = snapshot.selected_text.clone();
            tauri::async_runtime::spawn_blocking(move || transform_text(&settings, &mode, &source))
                .await
                .map_err(|error| format!("apply task failed: {error}"))??
        }
    };

    let adapter = WindowsAdapter::new();
    let receipt = adapter
        .write_back(&snapshot, &new_text)
        .map_err(|e| e.to_string())?;
    *state.last_receipt.safe_lock() = Some(receipt);
    Ok(())
}

/// Undo the last successful write.
#[tauri::command]
pub(crate) fn undo_last(state: State<AppState>) -> Result<(), String> {
    let receipt = state
        .last_receipt
        .lock()
        .unwrap()
        .clone()
        .ok_or("nothing to undo")?;
    let adapter = WindowsAdapter::new();
    adapter.undo(&receipt).map_err(|e| e.to_string())?;
    *state.last_receipt.safe_lock() = None;
    Ok(())
}
/// Shared capture flow for the Ctrl+Alt+Space hotkey and the manual button.
/// Returns whether a selection snapshot was captured and the panel opened.
fn capture_and_show(app: &AppHandle) -> bool {
    let adapter = WindowsAdapter::new();
    match adapter.capture_selection() {
        Ok(snapshot) => {
            let state = app.state::<AppState>();
            *state.snapshot.safe_lock() = Some(snapshot);
            state.selection_revision.fetch_add(1, Ordering::AcqRel);
            *state.last_preview.safe_lock() = None;
            if let Some(pet) = app.get_webview_window("pet") {
                let _ = pet.hide();
            }
            if let Some(window) = app.get_webview_window("main") {
                // Respect a position chosen by dragging. Before the first drag,
                // place the panel near the selection/cursor automatically.
                if !state.user_positioned.load(Ordering::Relaxed) {
                    position_overlay(app, &window);
                }
                let _ = window.show();
            }
            true
        }
        Err(error) => {
            eprintln!("capture failed: {error}");
            false
        }
    }
}

pub(crate) fn on_transform(app: &AppHandle) {
    capture_and_show(app);
}

/// Manual "capture selection" button. Safe to call while the panel is visible:
/// the rewrite view keeps the window non-activating (WS_EX_NOACTIVATE), so UIA
/// focus — and therefore the user's selection — stays in the source app.
#[tauri::command]
pub(crate) fn trigger_transform(app: AppHandle) -> bool {
    capture_and_show(&app)
}
pub(crate) fn on_undo(app: &AppHandle) {
    let state = app.state::<AppState>();
    let receipt = state.last_receipt.safe_lock().clone();
    if let Some(receipt) = receipt {
        let adapter = WindowsAdapter::new();
        let _ = adapter.undo(&receipt);
        *state.last_receipt.safe_lock() = None;
    }
}
