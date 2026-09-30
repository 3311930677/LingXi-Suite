//! 后台热键工作线程：改写/撤销热键循环 + 小工具/主面板全局快捷键消息泵。

use std::thread;
use std::time::Duration;

use assistant_windows::{run_assistant_hotkey_loop, wait_for_trigger_release, AssistantHotkey};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_BACK, VK_ESCAPE, VK_F1, VK_RETURN, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

use crate::rewrite::{on_transform, on_undo};
use crate::state::MutexExt;

/// B5-2：热键冲突提示事件（前端 showStatus 展示）。
pub(crate) const HOTKEY_CONFLICT_EVENT: &str = "lingxi://hotkey-conflict";

/// 解析 Win32 组合语法（`"Ctrl+Alt+D"` → `(modifiers, vk)`）：
/// 修饰键 Ctrl/Alt/Shift/Win（大小写不敏感、可任意组合），
/// 主键为单个字母/数字、F1~F24，或 space/enter/tab/backspace/esc。
/// 缺主键、多个主键、未知键名 → None（调用方报错或回落默认）。
pub(crate) fn parse_hotkey(text: &str) -> Option<(u32, u32)> {
    let mut modifiers: u32 = MOD_NOREPEAT.0;
    let mut main: Option<u32> = None;
    for part in text.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
            "alt" => modifiers |= MOD_ALT.0,
            "shift" => modifiers |= MOD_SHIFT.0,
            "win" | "super" | "meta" => modifiers |= MOD_WIN.0,
            other => {
                if main.is_some() {
                    return None; // 多个主键
                }
                main = Some(parse_main_key(other)?);
            }
        }
    }
    main.map(|vk| (modifiers, vk))
}

fn parse_main_key(key: &str) -> Option<u32> {
    if key.chars().count() == 1 {
        let ch = key.chars().next()?;
        return if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_uppercase() as u32) // ASCII 大写字母/数字即 VK 值
        } else {
            None
        };
    }
    if let Some(rest) = key.strip_prefix('f') {
        if let Ok(number) = rest.parse::<u32>() {
            if (1..=24).contains(&number) {
                return Some(VK_F1.0 as u32 + (number - 1));
            }
        }
    }
    match key {
        "space" => Some(VK_SPACE.0 as u32),
        "enter" | "return" => Some(VK_RETURN.0 as u32),
        "tab" => Some(VK_TAB.0 as u32),
        "backspace" => Some(VK_BACK.0 as u32),
        "escape" | "esc" => Some(VK_ESCAPE.0 as u32),
        _ => None,
    }
}
/// Background hotkey worker: capture on transform, revert on undo.
pub(crate) fn spawn_hotkey_worker(app: AppHandle) {
    thread::spawn(move || {
        let result = run_assistant_hotkey_loop(|command| {
            // Let Ctrl/Alt lift so injected keystrokes stay clean.
            wait_for_trigger_release(Duration::from_millis(800));
            match command {
                AssistantHotkey::Transform => on_transform(&app),
                AssistantHotkey::Undo => on_undo(&app),
            }
        });
        if let Err(error) = result {
            eprintln!("hotkey worker stopped: {error}");
        }
    });
}

/// B5-1：主面板呼出热键 ID（组合可配置，默认 Ctrl+Alt+D）。
/// 小工具全局热键已随「桌宠进度控制台」精简移除（托盘菜单入口同步删除）。
const PANEL_HK: i32 = 0x20;

/// 全局热键消息泵（后台线程）：注册主面板呼出热键并在 WM_HOTKEY 上分发。
pub(crate) fn spawn_global_hotkey_worker(app: AppHandle) {
    thread::spawn(move || {
        let mut registered: Vec<i32> = Vec::new();
        // B5-1/B5-2：主面板呼出热键（设置页可配；注册失败给出可见提示，不中断）。
        let panel_hotkey_text = app
            .state::<crate::state::AppState>()
            .backend
            .safe_lock()
            .hotkey_panel
            .clone();
        let panel_hotkey_text = if panel_hotkey_text.trim().is_empty() {
            "Ctrl+Alt+D".to_string()
        } else {
            panel_hotkey_text
        };
        match parse_hotkey(&panel_hotkey_text) {
            Some((mods, vk)) => {
                match unsafe { RegisterHotKey(None, PANEL_HK, HOT_KEY_MODIFIERS(mods), vk) } {
                    Ok(_) => registered.push(PANEL_HK),
                    Err(error) => {
                        eprintln!("[lingxi] panel hotkey register failed: {error}");
                        let _ = app.emit(
                            HOTKEY_CONFLICT_EVENT,
                            format!("主面板呼出热键 {panel_hotkey_text} 被其他程序占用，请在设置中更换"),
                        );
                    }
                }
            }
            None => {
                eprintln!("[lingxi] panel hotkey 配置无效：{panel_hotkey_text}");
                let _ = app.emit(
                    HOTKEY_CONFLICT_EVENT,
                    format!("热键配置无效：{panel_hotkey_text}（示例：Ctrl+Alt+D）"),
                );
            }
        }

        let mut msg = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if ret.0 <= 0 {
                break;
            }
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == PANEL_HK {
                // B5-1：主面板呼出/收起（与托盘「显示面板」同一条内部路径）。
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::panel::toggle_panel_inner(&app) {
                        eprintln!("[lingxi] toggle panel from hotkey: {e}");
                    }
                });
            }
        }

        for id in &registered {
            let _ = unsafe { UnregisterHotKey(None, *id) };
        }
    });
}
