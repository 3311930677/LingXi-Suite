//! Windows platform adapter: UIA / Win32 based reading and writing of the
//! foreground control's selection.
//!
//! Implements capability probing, selection capture, focus-drift protection,
//! transactional write-back, verification and undo.

mod capability;
mod clipboard;
mod com;
mod error;
mod foreground;
mod hotkey;
mod keyboard;
mod qq;
mod read;
mod snapshot;
mod uia;
mod writer;

use assistant_core::{
    AdapterError, CapabilityReport, InputAdapter, SelectionSnapshot, UndoReceipt, WriteReceipt,
};

pub use capability::FocusedProbe;
pub use clipboard::{read_clipboard_text, write_clipboard_text};
pub use foreground::{foreground_info, window_info, ForegroundInfo};
pub use hotkey::{run_assistant_hotkey_loop, run_hotkey_loop, AssistantHotkey};
pub use keyboard::{click_at, type_unicode, wait_for_trigger_release};
pub use qq::{
    capture_qq_selection_text, qq_latest_message, qq_write_draft, remember_foreground_if_qq,
    resolve_qq_window, QqMessageSnapshot,
};
pub use read::SelectionRead;

/// Insert Unicode text at the current caret without requiring a selected range.
///
/// Used by the keyboard-hook IME path: its candidate window never takes focus,
/// so the original editor remains foreground and a controlled paste inserts at
/// exactly that editor's caret. The clipboard's Unicode text is restored after
/// the paste. Unlike [`InputAdapter::write_back`], this intentionally has no
/// selection snapshot because a normal IME inserts into an empty caret.
pub fn insert_text_at_caret(text: &str) -> Result<(), AdapterError> {
    if text.is_empty() {
        return Ok(());
    }
    clipboard::paste_text_preserving_clipboard(text)
}

use com::ComGuard;
use uia::UiaClient;

/// Probe the currently focused control and return full diagnostics.
///
/// Initializes COM (MTA) for the duration of the call.
pub fn probe_focused() -> Result<FocusedProbe, AdapterError> {
    let _com = ComGuard::new()?;
    let client = UiaClient::new()?;
    let element = client.focused_element()?;
    capability::inspect(&element)
}

/// Concrete Windows implementation of [`InputAdapter`].
#[derive(Default)]
pub struct WindowsAdapter;

impl WindowsAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl InputAdapter for WindowsAdapter {
    fn probe_capability(&self) -> Result<CapabilityReport, AdapterError> {
        Ok(probe_focused()?.capability)
    }

    fn capture_selection(&self) -> Result<SelectionSnapshot, AdapterError> {
        let _com = ComGuard::new()?;
        let client = UiaClient::new()?;
        let element = client.focused_element()?;
        snapshot::capture(&element)
    }

    fn write_back(
        &self,
        target: &SelectionSnapshot,
        new_text: &str,
    ) -> Result<WriteReceipt, AdapterError> {
        let _com = ComGuard::new()?;
        let client = UiaClient::new()?;
        let element = client.focused_element()?;
        writer::write(&element, target, new_text)
    }

    fn undo(&self, receipt: &WriteReceipt) -> Result<UndoReceipt, AdapterError> {
        let _com = ComGuard::new()?;
        let client = UiaClient::new()?;
        let element = client.focused_element()?;
        writer::undo(&element, receipt)
    }
}
