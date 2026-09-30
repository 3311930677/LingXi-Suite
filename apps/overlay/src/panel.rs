//! 窗口原生行为：非激活样式（不抢焦点）、桌宠拖动、退出。

use tauri::{PhysicalPosition, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
};

/// Toggle the window's `WS_EX_NOACTIVATE` extended style and Tauri focusable
/// flag together. The non-activating state is the default: mouse clicks and
/// drag gestures still work, but keyboard/UIA focus stays in the source app.
fn set_window_activating(window: &WebviewWindow, activating: bool) -> Result<(), String> {
    window
        .set_focusable(activating)
        .map_err(|error| error.to_string())?;
    let tauri_hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let hwnd = HWND(tauri_hwnd.0);
    // SAFETY: `hwnd` belongs to this process and remains valid for the window's
    // lifetime. We preserve every existing extended style and flip one flag.
    unsafe {
        let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let updated = if activating {
            styles & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            styles | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated);
    }
    Ok(())
}

/// Prevent mouse interaction with the pet from activating it (no focus steal).
pub(crate) fn make_non_activating(window: &WebviewWindow) -> Result<(), String> {
    set_window_activating(window, false)
}

/// Move the pet window by a screen-space delta. Manual dragging keeps every
/// mouse event inside the WebView (`start_dragging` hands the mouse to the OS
/// modal drag loop, which swallows further JS events on non-activating
/// windows), so the front end drives position updates frame by frame instead.
#[tauri::command]
pub(crate) fn move_pet_by(window: WebviewWindow, dx: i32, dy: i32) -> Result<(), String> {
    if window.label() != "pet" {
        return Err("move_pet_by 仅限桌宠窗口".to_string());
    }
    if dx == 0 && dy == 0 {
        return Ok(());
    }
    let pos = window.outer_position().map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(pos.x + dx, pos.y + dy))
        .map_err(|error| error.to_string())
}

/// Quit the entire LingXi process (tray menu / pet menu).
#[tauri::command]
pub(crate) fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}
