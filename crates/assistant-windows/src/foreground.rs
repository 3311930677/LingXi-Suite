//! Foreground window inspection: which window is active, its title, and the
//! owning process image name. Used by W1 to know *where* the user is before we
//! ever touch the selection.
//!
//! None of these calls require COM; they are plain Win32.

use assistant_core::AdapterError;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};

/// Information about the currently active (foreground) window.
#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    /// Native window handle, kept as `isize` so core stays platform-agnostic.
    pub hwnd: isize,
    /// Process id owning the window.
    pub pid: u32,
    /// Window title (may be empty for some windows).
    pub title: String,
    /// Process image file name, e.g. "notepad.exe" (best-effort).
    pub process_name: String,
}

/// Inspect the current foreground window.
pub fn foreground_info() -> Result<ForegroundInfo, AdapterError> {
    // SAFETY: returns the foreground window handle or a null handle.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(AdapterError::NoFocusedElement);
    }
    Ok(describe_window(hwnd))
}

/// Inspect an arbitrary window by handle (not necessarily the foreground one).
///
/// Used by the QQ integration to act on a remembered QQ window after the panel
/// has stolen foreground focus. Returns `None` if the handle is null.
pub fn window_info(hwnd: isize) -> Option<ForegroundInfo> {
    if hwnd == 0 {
        return None;
    }
    // SAFETY: `HWND` is a plain handle wrapper; the accessor calls below tolerate
    // an invalid handle by returning empty/zero values.
    Some(describe_window(HWND(hwnd as *mut _)))
}

/// Gather title/pid/process-name for a window handle.
fn describe_window(hwnd: HWND) -> ForegroundInfo {
    let title = window_title(hwnd);
    let pid = window_process_id(hwnd);
    let process_name = if pid != 0 {
        process_image_name(pid).unwrap_or_default()
    } else {
        String::new()
    };

    ForegroundInfo {
        hwnd: hwnd.0 as isize,
        pid,
        title,
        process_name,
    }
}

/// Read a window's title text.
fn window_title(hwnd: HWND) -> String {
    // SAFETY: length query on a valid HWND.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    // +1 for the terminating NUL that GetWindowTextW writes.
    let mut buf = vec![0u16; len as usize + 1];
    // SAFETY: buffer is large enough for the reported length plus NUL.
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..written as usize])
}

/// Return the process id that owns `hwnd`, or 0 on failure.
fn window_process_id(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    // SAFETY: writes the owning process id into `pid`.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Resolve a process id to its executable file name (e.g. "notepad.exe").
///
/// Uses `PROCESS_QUERY_LIMITED_INFORMATION`, which succeeds for most processes
/// without elevation. Protected/system processes may still be inaccessible; in
/// that case the caller falls back to an empty string.
fn process_image_name(pid: u32) -> Option<String> {
    // SAFETY: opens a handle with the minimal query right; closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;

    let mut buf = vec![0u16; MAX_PATH as usize];
    let mut size = buf.len() as u32;
    // SAFETY: `buf`/`size` describe a valid writable buffer; `handle` is valid.
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
    };

    // SAFETY: balances the successful `OpenProcess` above.
    let _ = unsafe { CloseHandle(handle) };

    result.ok()?;
    let full = String::from_utf16_lossy(&buf[..size as usize]);
    // Keep only the file name component.
    let name = full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string();
    Some(name)
}
