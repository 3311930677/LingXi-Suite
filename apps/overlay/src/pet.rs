//! 桌宠命令：状态切换（Rust 侧统一仲裁）、皮肤热切换、气泡文案与可见性设置。

use tauri::{AppHandle, Emitter, Manager, State};

use crate::pet_skin;
use crate::settings::persist_backend_settings;
use crate::state::{AppState, MutexExt};

/// 桌宠状态广播事件名（M1：桌面壳统一状态源，前端只监听不再各自推导）。
pub(crate) const PET_STATUS_EVENT: &str = "owo://pet-status";

/// 状态优先级（M1 仲裁表）：alert > thinking > speaking > idle。
fn priority(status: &str) -> u8 {
    match status {
        "alert" => 3,
        "thinking" => 2,
        "speaking" => 1,
        _ => 0,
    }
}

/// Rust 侧统一仲裁并广播桌宠状态（内存态，**不落盘**——避免每帧写盘）。
///
/// 规则：
/// - `idle` 强制回落（回合结束 / 流异常）；
/// - 其余按优先级只升不降：`alert`（待审批）不会被随后的 `token_delta` 冲掉。
pub(crate) fn broadcast_pet_status(app: &AppHandle, state: &AppState, next: &str) {
    let changed = {
        let mut current = state.pet_status.safe_lock();
        let allow = next == "idle" || priority(next) >= priority(&current);
        if allow && current.as_str() != next {
            *current = next.to_string();
            true
        } else {
            false
        }
    };
    if changed {
        let _ = app.emit(PET_STATUS_EVENT, serde_json::json!({ "status": next }));
    }
}

/// 审批已响应：清除 alert 并回到 thinking（回合继续跑）。
pub(crate) fn clear_pet_alert(app: &AppHandle, state: &AppState) {
    let was_alert = {
        let mut current = state.pet_status.safe_lock();
        if current.as_str() == "alert" {
            *current = "thinking".to_string();
            true
        } else {
            false
        }
    };
    if was_alert {
        let _ = app.emit(PET_STATUS_EVENT, serde_json::json!({ "status": "thinking" }));
    }
}

#[tauri::command]
pub(crate) fn pet_status(state: State<AppState>) -> String {
    state.pet_status.safe_lock().clone()
}

#[tauri::command]
pub(crate) fn set_pet_status(
    app: AppHandle,
    state: State<AppState>,
    status: String,
) -> Result<(), String> {
    if !matches!(status.as_str(), "idle" | "thinking" | "speaking" | "alert") {
        return Err("invalid pet status".into());
    }
    // 轻环（选区改写/QQ 草稿）仍走本命令；经同一仲裁表广播，避免多源打架。
    broadcast_pet_status(&app, &state, &status);
    Ok(())
}

/// 面板按钮：显示/隐藏桌宠（不改动气泡覆盖与其他设置）。
#[tauri::command]
pub(crate) fn set_pet_visible(
    app: AppHandle,
    state: State<AppState>,
    visible: bool,
) -> Result<(), String> {
    let (skin_id, overrides) = {
        let mut settings = state.backend.safe_lock();
        settings.pet_visible = visible;
        persist_backend_settings(&settings)?;
        (
            settings.pet_skin.clone(),
            settings.pet_bubble_overrides.clone(),
        )
    };
    if let Some(pet) = app.get_webview_window("pet") {
        if visible {
            let _ = pet.show();
        } else {
            let _ = pet.hide();
        }
    }
    if let Ok(view) = pet_skin::view_for(&skin_id, &overrides, visible) {
        let _ = app.emit("pet-config-changed", &view);
    }
    Ok(())
}

/// 设置页：列出全部可用皮肤。
#[tauri::command]
pub(crate) fn list_pet_skins() -> Vec<pet_skin::PetSkinInfo> {
    pet_skin::list_skins()
}

/// 桌宠窗口启动时拉取当前配置（皮肤 + 气泡覆盖 + 可见性）。
#[tauri::command]
pub(crate) fn current_pet_config(state: State<AppState>) -> Result<pet_skin::PetSkinView, String> {
    let settings = state.backend.safe_lock();
    pet_skin::view_for(
        &settings.pet_skin,
        &settings.pet_bubble_overrides,
        settings.pet_visible,
    )
}

/// 切换皮肤：校验 → 落盘 → 广播 `pet-config-changed`（桌宠窗口即时热换）。
#[tauri::command]
pub(crate) fn set_pet_skin(
    app: AppHandle,
    state: State<AppState>,
    skin_id: String,
) -> Result<pet_skin::PetSkinView, String> {
    if !pet_skin::valid_skin_id(&skin_id) {
        return Err("皮肤 id 非法".into());
    }
    let view = {
        let mut settings = state.backend.safe_lock();
        // 先校验皮肤可用，失败不落盘；切换不改变可见性设置。
        pet_skin::load_manifest(&skin_id)?;
        let visible = settings.pet_visible;
        let overrides = settings.pet_bubble_overrides.clone();
        settings.pet_skin = skin_id.clone();
        persist_backend_settings(&settings)?;
        drop(settings);
        pet_skin::view_for(&skin_id, &overrides, visible)?
    };
    app.emit("pet-config-changed", &view)
        .map_err(|e| e.to_string())?;
    Ok(view)
}

/// 桌宠杂项设置：气泡文案覆盖 + 可见性开关。
#[tauri::command]
pub(crate) fn set_pet_options(
    app: AppHandle,
    state: State<AppState>,
    overrides: pet_skin::PetBubbleOverrides,
    visible: bool,
) -> Result<pet_skin::PetSkinView, String> {
    let view = {
        let mut settings = state.backend.safe_lock();
        // 覆盖文案裁剪：空串视为"用皮肤默认"。
        let trimmed = |text: &Option<String>| {
            text.as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        settings.pet_bubble_overrides = pet_skin::PetBubbleOverrides {
            idle: trimmed(&overrides.idle),
            thinking: trimmed(&overrides.thinking),
            speaking: trimmed(&overrides.speaking),
            alert: trimmed(&overrides.alert),
        };
        settings.pet_visible = visible;
        persist_backend_settings(&settings)?;
        let skin_id = settings.pet_skin.clone();
        let overrides = settings.pet_bubble_overrides.clone();
        drop(settings);
        pet_skin::view_for(&skin_id, &overrides, visible)?
    };
    if visible {
        if let Some(pet) = app.get_webview_window("pet") {
            let _ = pet.show();
        }
    } else if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.hide();
    }
    app.emit("pet-config-changed", &view)
        .map_err(|e| e.to_string())?;
    Ok(view)
}
