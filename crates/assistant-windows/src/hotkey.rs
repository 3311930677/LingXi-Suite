//! Global hotkey registration and the message pump that drives it.
//!
//! W1 wires `Ctrl+Alt+Space` to a callback. The hotkey is *global* (works while
//! any application is focused) yet does not steal focus, which is exactly what
//! we need: the user keeps their selection in the target app while we react.
//!
//! `RegisterHotKey` associates the hotkey with the calling thread when no HWND
//! is given, so registration and the message loop must live on the same thread.

use assistant_core::AdapterError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_BACK, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

use crate::error::platform;

/// Identifier for our single hotkey registration.
const HOTKEY_ID: i32 = 1;
const UNDO_HOTKEY_ID: i32 = 2;

/// Commands emitted by the full assistant hotkey loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantHotkey {
    Transform,
    Undo,
}

/// Register `Ctrl+Alt+Space` and pump messages, invoking `on_trigger` on each
/// press. Blocks until the message loop ends (e.g. `WM_QUIT`) and then
/// unregisters the hotkey.
///
/// Must be called on the same thread that will own the message loop.
pub fn run_hotkey_loop<F: FnMut()>(mut on_trigger: F) -> Result<(), AdapterError> {
    let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;

    // SAFETY: `None` HWND binds the hotkey to the current thread; balanced by
    // `UnregisterHotKey` below.
    unsafe { RegisterHotKey(None, HOTKEY_ID, modifiers, VK_SPACE.0 as u32) }
        .map_err(|e| platform("RegisterHotKey(Ctrl+Alt+Space)", e))?;

    let mut msg = MSG::default();
    loop {
        // SAFETY: standard blocking message pump; `msg` is a valid out param.
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        // 0 => WM_QUIT, -1 => error. Either way, stop pumping.
        if ret.0 <= 0 {
            break;
        }
        if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
            on_trigger();
        }
    }

    // SAFETY: balances the successful `RegisterHotKey` above.
    let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
    Ok(())
}

/// Register the demonstration pipeline hotkeys:
/// - Ctrl+Alt+Space: capture and transform
/// - Ctrl+Alt+Backspace: undo the last successful write
pub fn run_assistant_hotkey_loop<F: FnMut(AssistantHotkey)>(
    mut on_command: F,
) -> Result<(), AdapterError> {
    let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
    unsafe { RegisterHotKey(None, HOTKEY_ID, modifiers, VK_SPACE.0 as u32) }
        .map_err(|e| platform("RegisterHotKey(Ctrl+Alt+Space)", e))?;
    if let Err(error) = unsafe { RegisterHotKey(None, UNDO_HOTKEY_ID, modifiers, VK_BACK.0 as u32) }
    {
        let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
        return Err(platform("RegisterHotKey(Ctrl+Alt+Backspace)", error));
    }
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }
        if msg.message == WM_HOTKEY {
            match msg.wParam.0 as i32 {
                HOTKEY_ID => on_command(AssistantHotkey::Transform),
                UNDO_HOTKEY_ID => on_command(AssistantHotkey::Undo),
                _ => {}
            }
        }
    }

    let _ = unsafe { UnregisterHotKey(None, UNDO_HOTKEY_ID) };
    let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
    Ok(())
}
