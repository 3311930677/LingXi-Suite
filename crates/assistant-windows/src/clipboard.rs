//! Transactional clipboard access using only the manual Win32 clipboard API.
//!
//! We deliberately do NOT mix the OLE clipboard API (`OleGetClipboard` /
//! `OleSetClipboard`) with the manual API: the two maintain independent
//! ownership state and interleaving them yields `CLIPBRD_E_CANT_CLOSE`
//! (0x800401D4). Everything here goes through Open/Get/Set/CloseClipboard.
//!
//! Scope: the original Unicode text (`CF_UNICODETEXT`) is preserved and
//! restored. Non-text formats (bitmaps, rich data) are not retained during the
//! brief copy/paste window; this is acceptable for a text input assistant and
//! avoids the fragility of deep-copying arbitrary clipboard formats.

use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use std::thread::sleep;
use std::time::{Duration, Instant};

use assistant_core::AdapterError;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::error::platform;
use crate::keyboard;

const OPEN_RETRIES: usize = 20;
const COPY_TIMEOUT: Duration = Duration::from_millis(750);
const PASTE_SETTLE: Duration = Duration::from_millis(120);

/// RAII guard around a successful `OpenClipboard`.
struct OpenClipboardGuard;

impl OpenClipboardGuard {
    fn acquire() -> Result<Self, AdapterError> {
        for _ in 0..OPEN_RETRIES {
            // SAFETY: null owner is valid for short synchronous access.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self);
            }
            sleep(Duration::from_millis(10));
        }
        Err(AdapterError::Platform(
            "OpenClipboard remained busy".to_string(),
        ))
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: only constructed after OpenClipboard succeeded.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Copy the active selection, read it, then restore the original clipboard
/// text. Used only when UIA exposes no readable pattern.
pub(crate) fn copy_selected_text() -> Result<String, AdapterError> {
    let backup = backup_text();
    // SAFETY: sequence query has no preconditions.
    let before = unsafe { GetClipboardSequenceNumber() };

    let operation = (|| {
        keyboard::copy()?;
        let deadline = Instant::now() + COPY_TIMEOUT;
        while Instant::now() < deadline {
            // SAFETY: sequence query has no preconditions.
            if unsafe { GetClipboardSequenceNumber() } != before {
                sleep(Duration::from_millis(20));
                let text = read_unicode_text()?;
                return if text.is_empty() {
                    Err(AdapterError::NoSelection)
                } else {
                    Ok(text)
                };
            }
            sleep(Duration::from_millis(10));
        }
        Err(AdapterError::NoSelection)
    })();

    let restored = restore_text(backup.as_deref());
    restored.and(operation)
}

/// Temporarily place `text` on the clipboard, paste it, then restore the
/// original clipboard text.
pub(crate) fn paste_text_preserving_clipboard(text: &str) -> Result<(), AdapterError> {
    let backup = backup_text();

    let operation = (|| {
        write_unicode_text(text)?;
        keyboard::paste()?;
        sleep(PASTE_SETTLE);
        Ok(())
    })();

    let restored = restore_text(backup.as_deref());
    restored.and(operation)
}

/// Snapshot the current Unicode clipboard text, or `None` when absent.
fn backup_text() -> Option<String> {
    // SAFETY: format availability query has no preconditions.
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.is_err() {
        return None;
    }
    read_unicode_text().ok().filter(|s| !s.is_empty())
}

/// Restore a previous text snapshot, or clear the clipboard when there was no
/// text originally (we may have overwritten it during the transaction).
fn restore_text(backup: Option<&str>) -> Result<(), AdapterError> {
    match backup {
        Some(text) => write_unicode_text(text),
        None => {
            let _open = OpenClipboardGuard::acquire()?;
            // SAFETY: clipboard is open on this thread.
            unsafe { EmptyClipboard() }.map_err(|e| platform("EmptyClipboard(restore)", e))
        }
    }
}

/// Read the current clipboard text. Returns an empty string if the clipboard
/// does not contain text (e.g. it holds an image).
pub fn read_clipboard_text() -> Result<String, AdapterError> {
    read_unicode_text()
}

/// Set the clipboard to the given text, replacing any previous content.
pub fn write_clipboard_text(text: &str) -> Result<(), AdapterError> {
    write_unicode_text(text)
}

fn read_unicode_text() -> Result<String, AdapterError> {
    let _open = OpenClipboardGuard::acquire()?;
    // SAFETY: clipboard is open; handle stays clipboard-owned.
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }
        .map_err(|e| platform("GetClipboardData(CF_UNICODETEXT)", e))?;
    let memory = HGLOBAL(handle.0);
    // SAFETY: handle refers to a global-memory clipboard block.
    let ptr = unsafe { GlobalLock(memory) } as *const u16;
    if ptr.is_null() {
        return Err(AdapterError::Platform("GlobalLock returned null".into()));
    }
    // SAFETY: valid global-memory block.
    let units = unsafe { GlobalSize(memory) } / size_of::<u16>();
    // SAFETY: locked block holds `units` readable u16 values.
    let slice = unsafe { std::slice::from_raw_parts(ptr, units) };
    let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    let text = String::from_utf16_lossy(&slice[..len]);
    // SAFETY: balances the GlobalLock above.
    let _ = unsafe { GlobalUnlock(memory) };
    Ok(text)
}

fn write_unicode_text(text: &str) -> Result<(), AdapterError> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let bytes = utf16.len() * size_of::<u16>();

    // SAFETY: allocates a movable block as required by SetClipboardData.
    let memory =
        unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map_err(|e| platform("GlobalAlloc", e))?;
    // SAFETY: newly allocated block is writable.
    let ptr = unsafe { GlobalLock(memory) } as *mut u16;
    if ptr.is_null() {
        // SAFETY: clipboard has not taken ownership yet.
        let _ = unsafe { GlobalFree(memory) };
        return Err(AdapterError::Platform("GlobalLock returned null".into()));
    }
    // SAFETY: destination has exactly `utf16.len()` u16 slots.
    unsafe { copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len()) };
    // SAFETY: balances the GlobalLock above.
    let _ = unsafe { GlobalUnlock(memory) };

    let set_result = (|| {
        let _open = OpenClipboardGuard::acquire()?;
        // SAFETY: clipboard is open on this thread.
        unsafe { EmptyClipboard() }.map_err(|e| platform("EmptyClipboard", e))?;
        // SAFETY: on success, ownership of `memory` transfers to the system.
        unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(memory.0)) }
            .map_err(|e| platform("SetClipboardData(CF_UNICODETEXT)", e))?;
        Ok(())
    })();

    if set_result.is_err() {
        // SAFETY: ownership was not transferred on failure.
        let _ = unsafe { GlobalFree(memory) };
    }
    set_result
}

#[cfg(test)]
mod tests {
    #[test]
    fn unicode_round_trip_representation() {
        let source = "中文 😀\r\nnext";
        let encoded: Vec<u16> = source.encode_utf16().collect();
        assert_eq!(String::from_utf16(&encoded).unwrap(), source);
    }
}
