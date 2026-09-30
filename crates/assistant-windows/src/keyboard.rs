//! Minimal, auditable keyboard injection used only after target validation.

use std::mem::size_of;
use std::thread::sleep;
use std::time::{Duration, Instant};

use assistant_core::AdapterError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEINPUT, VIRTUAL_KEY, VK_A, VK_C, VK_CONTROL,
    VK_DELETE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// Modifier keys that could still be physically held when a hotkey fires and
/// would otherwise contaminate our injected shortcuts (e.g. a lingering Alt
/// turning an injected Ctrl+V into Ctrl+Alt+V and triggering a third-party
/// global hotkey such as QQ's panel).
const STRAY_MODIFIERS: [VIRTUAL_KEY; 11] = [
    VK_CONTROL,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_MENU,
    VK_LMENU,
    VK_RMENU,
    VK_SHIFT,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LWIN,
    VK_RWIN,
];

/// Synthesize key-up events for every modifier so the following injection is a
/// clean chord regardless of what the user is still physically pressing. The
/// OS will emit its own key-up when the physical key is eventually released,
/// which is harmless.
fn release_stray_modifiers() -> Result<(), AdapterError> {
    let inputs: Vec<INPUT> = STRAY_MODIFIERS
        .iter()
        .map(|key| key_input(*key, 0, KEYEVENTF_KEYUP))
        .collect();
    send_all(&inputs)?;
    // Give the target a moment to process the key-ups before the real chord.
    sleep(Duration::from_millis(15));
    Ok(())
}

/// Wait until the trigger chord is physically released. `WM_HOTKEY` can arrive
/// while Ctrl/Alt are still held; injecting another shortcut before release
/// would create an unintended Ctrl+Alt combination in the target app.
pub fn wait_for_trigger_release(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: key-state queries have no memory preconditions.
        let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        // SAFETY: key-state queries have no memory preconditions.
        let alt = unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } < 0;
        if !ctrl && !alt {
            return;
        }
        sleep(Duration::from_millis(5));
    }
}

pub(crate) fn copy() -> Result<(), AdapterError> {
    send_ctrl_key(VK_C)
}

pub(crate) fn paste() -> Result<(), AdapterError> {
    send_ctrl_key(VK_V)
}

pub(crate) fn select_all() -> Result<(), AdapterError> {
    send_ctrl_key(VK_A)
}

pub(crate) fn undo() -> Result<(), AdapterError> {
    send_ctrl_key(VK_Z)
}

pub(crate) fn delete_selection() -> Result<(), AdapterError> {
    release_stray_modifiers()?;
    let inputs = [
        key_input(VK_DELETE, 0, Default::default()),
        key_input(VK_DELETE, 0, KEYEVENTF_KEYUP),
    ];
    send_all(&inputs)
}

/// Move the cursor to (x, y) in screen coordinates and perform a single left
/// click. Used by the QQ write path to drive the caret into the Chromium
/// composer when UIA cannot identify the editable element directly: QQNT only
/// renders the composer's accessibility node after it receives focus, so we
/// click on its physical location (computed from the QQ window bounds) and
/// then use Ctrl+A / Ctrl+V to replace its contents.
pub fn click_at(x: i32, y: i32) -> Result<(), AdapterError> {
    // Convert screen pixels to the absolute coordinate space SendInput expects
    // (0..65535 mapped across the full primary monitor). Doing the move and
    // click in a single SendInput batch avoids the cursor visibly dwelling at
    // the old position.
    // SAFETY: GetSystemMetrics only reads a system-wide int.
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1);
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1);
    let abs_x = ((x as i64 * 65535) / screen_w as i64) as i32;
    let abs_y = ((y as i64 * 65535) / screen_h as i64) as i32;

    let move_and_down = mouse_input_abs(
        abs_x,
        abs_y,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_LEFTDOWN,
    );
    let up = mouse_input_abs(0, 0, MOUSEEVENTF_LEFTUP);
    send_all_mouse(&[move_and_down, up])?;
    // Let the focus event propagate through Chromium's accessibility tree
    // before the caller injects Ctrl+A / Ctrl+V.
    sleep(Duration::from_millis(120));
    Ok(())
}

fn mouse_input_abs(
    dx: i32,
    dy: i32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_all_mouse(inputs: &[INPUT]) -> Result<(), AdapterError> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: `inputs` is a valid contiguous INPUT array and cbSize matches.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(AdapterError::Platform(format!(
            "SendInput(mouse) sent {sent}/{} events",
            inputs.len()
        )));
    }
    Ok(())
}

/// Type arbitrary UTF-16 text into the active selection using KEYEVENTF_UNICODE.
pub fn type_unicode(text: &str) -> Result<(), AdapterError> {
    release_stray_modifiers()?;
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for code_unit in text.encode_utf16() {
        inputs.push(key_input(VIRTUAL_KEY(0), code_unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(
            VIRTUAL_KEY(0),
            code_unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }
    send_all(&inputs)
}

fn send_ctrl_key(key: VIRTUAL_KEY) -> Result<(), AdapterError> {
    // Clear any physically-held modifiers first, then press only the Ctrl we
    // need, so the target sees exactly Ctrl+<key> and nothing else.
    release_stray_modifiers()?;
    let inputs = [
        key_input(VK_CONTROL, 0, Default::default()),
        key_input(key, 0, Default::default()),
        key_input(key, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    send_all(&inputs)
}

fn key_input(
    virtual_key: VIRTUAL_KEY,
    scan_code: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_all(inputs: &[INPUT]) -> Result<(), AdapterError> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: `inputs` is a valid contiguous INPUT array and cbSize matches.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(AdapterError::Platform(format!(
            "SendInput sent {sent}/{} events",
            inputs.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn utf16_event_count_handles_surrogate_pairs() {
        let text = "A中😀";
        assert_eq!(text.encode_utf16().count(), 4);
    }
}
