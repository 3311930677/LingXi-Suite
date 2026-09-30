//! 面板生命周期：显示/隐藏、托盘退出、失焦自动收起、拖动/缩放、焦点控制。

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, State, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, SendMessageW,
    SetWindowLongPtrW, GWL_EXSTYLE, HTBOTTOMRIGHT, WM_NCLBUTTONDOWN, WS_EX_NOACTIVATE,
};

use crate::placement::position_overlay;
use crate::state::{AppState, MutexExt};
/// 主面板开关的内部实现（Tauri 命令与 B5-1 全局热键共用；返回是否变为可见）。
pub(crate) fn toggle_panel_inner(app: &AppHandle) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("panel window is unavailable")?;
    let visible = window.is_visible().map_err(|error| error.to_string())?;
    if visible {
        hide_panel_quietly(app);
        Ok(false)
    } else {
        // Keep the two always-on-top windows from covering each other. The pet
        // returns as soon as the panel is closed.
        if pet_allowed(app) {
            if let Some(pet) = app.get_webview_window("pet") {
                let _ = pet.hide();
            }
        }
        // 尊重用户拖动过的位置；从未拖动时才跟随光标出现。
        if !app
            .state::<AppState>()
            .user_positioned
            .load(Ordering::Relaxed)
        {
            position_overlay(app, &window);
        }
        window.show().map_err(|error| error.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
pub(crate) fn toggle_panel(app: AppHandle) -> Result<bool, String> {
    toggle_panel_inner(&app)
}
/// 收起主面板的统一出口：隐藏面板、按设置放回桌宠、桌宠状态归位。
/// 关闭按钮/Esc/失焦自动收起共用，保证行为一致。
fn hide_panel_quietly(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.hide() {
            eprintln!("[lingxi] hide panel failed: {error}");
        }
    }
    if pet_allowed(app) {
        if let Some(pet) = app.get_webview_window("pet") {
            let _ = pet.show();
        }
    }
    let state = app.state::<AppState>();
    *state.pet_status.safe_lock() = "idle".into();
}

fn pet_allowed(app: &AppHandle) -> bool {
    app.state::<AppState>().backend.safe_lock().pet_visible
}

/// Hide the overlay (called by the close button / Esc).
#[tauri::command]
pub(crate) fn hide_overlay(app: AppHandle) {
    hide_panel_quietly(&app);
}

/// 面板失焦自动收起。面板默认是非激活窗口（不抢焦点），拿不到可靠的
/// blur 事件，因此用前台窗口轮询判断"用户是否已切去别的应用"：
/// 记住面板弹出时的前台窗口为锚点；前台换成其他进程的窗口并停留约
/// 1.2 秒，视为用户已离开，自动收起面板。焦点在本进程任何窗口
/// （面板输入态/桌宠/小组件/候选窗）时永不收起。
pub(crate) fn spawn_panel_autohide_worker(app: AppHandle) {
    thread::spawn(move || {
        let mut prev_visible = false;
        let mut anchor: isize = 0;
        let mut misses: u32 = 0;
        loop {
            thread::sleep(Duration::from_millis(400));
            let Some(panel) = app.get_webview_window("main") else {
                continue;
            };
            let Ok(visible) = panel.is_visible() else {
                continue;
            };
            let auto_hide = app.state::<AppState>().backend.safe_lock().panel_auto_hide;
            if !visible || !auto_hide {
                if prev_visible {
                    eprintln!(
                        "[lingxi] autohide: disengaged (visible={visible}, auto_hide={auto_hide})"
                    );
                }
                prev_visible = false;
                anchor = 0;
                misses = 0;
                continue;
            }
            // SAFETY: GetForegroundWindow is thread-safe and never blocks.
            let fg = unsafe { GetForegroundWindow() }.0 as isize;
            if !prev_visible {
                // 刚显示：锚定用户此刻所在的应用。
                prev_visible = true;
                anchor = if is_our_process_hwnd(fg) { 0 } else { fg };
                misses = 0;
                eprintln!("[lingxi] autohide: engaged, anchor={anchor:#x}");
                continue;
            }
            if fg == 0 || is_our_process_hwnd(fg) {
                misses = 0;
                continue;
            }
            if anchor == 0 {
                anchor = fg;
                misses = 0;
                eprintln!("[lingxi] autohide: late anchor={anchor:#x}");
                continue;
            }
            if fg == anchor {
                misses = 0;
                continue;
            }
            misses += 1;
            eprintln!("[lingxi] autohide: miss {misses}, fg={fg:#x}, anchor={anchor:#x}");
            if misses >= 3 {
                eprintln!("[lingxi] autohide: foreground left anchor for 1.2s, hiding panel");
                hide_panel_quietly(&app);
                thread::sleep(Duration::from_millis(250));
                if let Some(panel) = app.get_webview_window("main") {
                    eprintln!(
                        "[lingxi] autohide: post-hide visible={:?}",
                        panel.is_visible()
                    );
                }
                prev_visible = false;
                anchor = 0;
                misses = 0;
            }
        }
    });
}

fn is_our_process_hwnd(raw: isize) -> bool {
    if raw == 0 {
        return false;
    }
    let mut pid: u32 = 0;
    // SAFETY: `HWND` is a plain handle value; GetWindowThreadProcessId only reads it.
    unsafe { GetWindowThreadProcessId(HWND(raw as _), Some(&mut pid)) };
    pid == std::process::id()
}

/// Quit the entire LingXi process (called by the "退出灵犀" button).
/// Unlike `hide_overlay` which only hides the panel, this fully exits the
/// application so the user does not need to find the hidden tray icon or use
/// Task Manager.
#[tauri::command]
pub(crate) fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Explicitly begin a native window drag. This custom command bypasses the
/// window-plugin permission path used by `data-tauri-drag-region`, which is
/// unreliable for a non-activating WebView window.
#[tauri::command]
pub(crate) fn start_window_drag(
    window: WebviewWindow,
    state: State<AppState>,
) -> Result<(), String> {
    // 只有面板被拖动才锁定位置；拖桌宠不应影响面板的跟随光标行为。
    if window.label() == "main" {
        state.user_positioned.store(true, Ordering::Relaxed);
    }
    window.start_dragging().map_err(|error| error.to_string())
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

/// Begin native south-east resize dragging from the visible corner grip. A
/// WebviewWindow does not expose Tauri's Window-only resize method, so send the
/// standard non-client hit-test message directly to Win32.
#[tauri::command]
pub(crate) fn start_window_resize(window: WebviewWindow) -> Result<(), String> {
    let raw = window.hwnd().map_err(|error| error.to_string())?.0;
    let hwnd = HWND(raw);
    // SAFETY: `hwnd` belongs to this live Tauri window. Releasing the current
    // mouse capture and sending WM_NCLBUTTONDOWN/HTBOTTOMRIGHT delegates the
    // resize loop to Windows exactly like dragging a decorated window corner.
    unsafe {
        let _ = ReleaseCapture();
        let _ = SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            windows::Win32::Foundation::WPARAM(HTBOTTOMRIGHT as usize),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
    Ok(())
}
/// Toggle the overlay's `WS_EX_NOACTIVATE` extended style and Tauri focusable
/// flag together. The non-activating state is the default: mouse clicks and drag
/// gestures still work, but keyboard/UIA focus stays in the source editor so
/// snapshot validation and write-back remain safe. It is temporarily lifted when
/// a view needs real text entry (see `set_panel_focusable`).
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

/// Prevent mouse interaction with the overlay from activating it. See
/// `set_window_activating` for the rationale.
pub(crate) fn make_non_activating(window: &WebviewWindow) -> Result<(), String> {
    set_window_activating(window, false)
}

/// Let the panel accept keyboard focus so text fields (API key, QQ draft) can be
/// typed into, or restore the non-activating default. Views that require typing
/// call this with `true` on entry and `false` when leaving; the write-back path
/// therefore keeps running against a non-activating window as before.
#[tauri::command]
pub(crate) fn set_panel_focusable(app: AppHandle, focusable: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("panel window is unavailable")?;
    set_window_activating(&window, focusable)?;
    if focusable {
        // Actually pull focus to the window; otherwise the freshly focusable
        // window still has no keyboard focus and the caret never appears.
        window.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}
