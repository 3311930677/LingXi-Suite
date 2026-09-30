//! Automated native integration smoke test.
//!
//! Creates a real Win32 Edit control, selects Unicode text, then exercises the
//! production UIA capture -> write -> verify -> undo path without human input.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use assistant_core::InputAdapter;
use assistant_windows::{foreground_info, WindowsAdapter};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, PostMessageW, PostThreadMessageW, SendMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage, HMENU, MSG, SW_SHOW,
    WINDOW_EX_STYLE, WM_CLOSE, WM_QUIT, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const EM_SETSEL: u32 = 0x00B1;
const ORIGINAL: &str = "alpha beta中文 gamma";
const SELECTED: &str = "beta中文";

fn main() {
    let (window, edit) = create_test_window().expect("create Win32 test control");
    let _ = unsafe { SetForegroundWindow(window) };
    unsafe { SetFocus(edit) }.expect("focus Edit control");
    // EM_SETSEL uses UTF-16 code-unit offsets. The selected range starts after
    // ASCII "alpha " and contains four ASCII plus two BMP characters.
    unsafe { SendMessageW(edit, EM_SETSEL, WPARAM(6), LPARAM(12)) };
    thread::sleep(Duration::from_millis(100));
    let is_our_foreground = foreground_info()
        .map(|info| info.hwnd == window.0 as isize)
        .unwrap_or(false);
    if !is_our_foreground {
        println!("SMOKE SKIP: execution session cannot foreground the native test window");
        return;
    }

    let (tx, rx) = mpsc::channel();
    let window_value = window.0 as usize;
    // SAFETY: returns the numeric id of this UI/message-loop thread.
    let ui_thread_id = unsafe { GetCurrentThreadId() };
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        let result = run_pipeline();
        let _ = tx.send(result);
        let window = HWND(window_value as *mut _);
        let _ = unsafe { PostMessageW(window, WM_CLOSE, WPARAM(0), LPARAM(0)) };
        let _ = unsafe { PostThreadMessageW(ui_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    });

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        let _ = unsafe { TranslateMessage(&msg) };
        unsafe { DispatchMessageW(&msg) };
    }

    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(summary)) => println!("SMOKE PASS: {summary}"),
        Ok(Err(error)) => {
            eprintln!("SMOKE FAIL: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("SMOKE FAIL: worker result unavailable: {error}");
            std::process::exit(1);
        }
    }
}

fn run_pipeline() -> Result<String, String> {
    let adapter = WindowsAdapter::new();
    let snapshot = adapter.capture_selection().map_err(|e| e.to_string())?;
    if snapshot.selected_text != SELECTED {
        return Err(format!(
            "selection mismatch: expected {SELECTED:?}, got {:?}",
            snapshot.selected_text
        ));
    }
    let transformed = format!("[AI] {SELECTED}");
    let receipt = adapter
        .write_back(&snapshot, &transformed)
        .map_err(|e| e.to_string())?;
    if !receipt.verified {
        return Err("write completed but verification was false".into());
    }
    let undo = adapter.undo(&receipt).map_err(|e| e.to_string())?;
    if !undo.verified {
        return Err("undo completed but verification was false".into());
    }
    Ok(format!(
        "captured={SELECTED:?}, strategy={:?}, write_verified=true, undo_verified=true",
        receipt.strategy_used
    ))
}

fn create_test_window() -> windows::core::Result<(HWND, HWND)> {
    // Use predefined system classes, so no custom WndProc registration is
    // required for this short-lived integration harness.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("cross-app-assistant smoke"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            640,
            180,
            None,
            None,
            None,
            None,
        )
    }?;
    let edit = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            20,
            40,
            580,
            32,
            window,
            HMENU::default(),
            None,
            None,
        )
    }?;
    unsafe { SetWindowTextW(edit, &windows::core::HSTRING::from(ORIGINAL)) }?;
    let _ = unsafe { ShowWindow(window, SW_SHOW) };
    Ok((window, edit))
}
