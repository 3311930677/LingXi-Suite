//! 窗口摆放与位置持久化：光标跟随、工作区夹取、拖动落点记录与防抖落盘。

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::state::{AppState, MutexExt};
use crate::window_state;
/// Position the overlay near the cursor, fully inside the work area of the
/// monitor under the cursor. Config `center: true` is unreliable for a
/// transparent, borderless window (its size isn't settled at creation), so we
/// place it explicitly on every show. Win32 (physical px), the monitor work
/// area, and Tauri's PhysicalPosition all agree under per-monitor DPI.
pub(crate) fn position_overlay(app: &AppHandle, window: &WebviewWindow) {
    let mut cursor = POINT::default();
    // SAFETY: GetCursorPos writes the current cursor position into `cursor`.
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        let _ = window.center();
        return;
    }

    // SAFETY: MonitorFromPoint always returns a valid monitor with NEAREST.
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` is set as required by GetMonitorInfoW.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let _ = window.center();
        return;
    }

    let work = info.rcWork;
    let size = window.outer_size().unwrap_or(PhysicalSize::new(520, 520));
    let w = size.width as i32;
    let h = size.height as i32;

    // Offset a little from the caret so the panel doesn't cover it, then clamp
    // the whole window inside the work area.
    let x = (cursor.x + 24).min(work.right - w).max(work.left);
    let y = (cursor.y + 24).min(work.bottom - h).max(work.top);
    set_position_tracked(app, window, x, y);
}

pub(crate) fn position_pet(app: &AppHandle, window: &WebviewWindow) {
    // Place the whole pet inside the monitor work area (excluding taskbar).
    // `current_monitor().size()` includes the taskbar and could leave the lower
    // torso clipped, especially under Windows display scaling.
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return;
    }
    let work = info.rcWork;
    let size = window.outer_size().unwrap_or(PhysicalSize::new(220, 260));
    let x = (work.right - size.width as i32 - 24).max(work.left);
    let y = (work.bottom - size.height as i32 - 24).max(work.top);
    set_position_tracked(app, window, x, y);
}

/// 程序化摆放窗口：先记录目标位置，Moved 事件与它一致时视为程序摆放，
/// 不当作用户拖动写入持久化。
pub(crate) fn set_position_tracked(app: &AppHandle, window: &WebviewWindow, x: i32, y: i32) {
    let state = app.state::<AppState>();
    state
        .last_programmatic_pos
        .safe_lock()
        .insert(window.label().to_string(), PhysicalPosition::new(x, y));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// 窗口中心是否落在某块已连接的显示器上（拔掉显示器后旧坐标失效）。
pub(crate) fn position_on_screen(x: i32, y: i32, w: i32, h: i32) -> bool {
    if w <= 0 || h <= 0 {
        return false;
    }
    let center = POINT {
        x: x + w / 2,
        y: y + h / 2,
    };
    // SAFETY: MonitorFromPoint with DEFAULTTONULL only reads the point value.
    let monitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONULL) };
    !monitor.is_invalid()
}

/// 用户拖动后的落点：写入内存状态并防抖落盘。拖动过程会连续触发 Moved，
/// 防抖保证一次拖动最终只写一次盘（trailing edge，不丢最终位置）。
pub(crate) fn handle_window_moved(app: &AppHandle, label: &str, pos: PhysicalPosition<i32>) {
    let state = app.state::<AppState>();
    {
        let programmatic = state.last_programmatic_pos.safe_lock();
        if programmatic.get(label) == Some(&pos) {
            return;
        }
    }
    let mut windows = state.window_state.safe_lock();
    let entry = window_state::WindowPos { x: pos.x, y: pos.y };
    if label == "main" {
        windows.panel = Some(entry);
        state.user_positioned.store(true, Ordering::Relaxed);
    } else {
        windows.pet = Some(entry);
    }
    drop(windows);
    schedule_window_state_persist(app);
}

pub(crate) fn schedule_window_state_persist(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.window_save_pending.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        let state = app.state::<AppState>();
        if state.window_save_pending.swap(false, Ordering::SeqCst) {
            let snapshot = state.window_state.safe_lock().clone();
            if let Err(error) = window_state::persist(&snapshot) {
                eprintln!("[lingxi] window_state persist failed: {error}");
            }
        }
    });
}
