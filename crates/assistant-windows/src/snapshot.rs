//! Capture and validate the target identity used by safe write-back.

use std::time::Instant;

use assistant_core::{AdapterError, ReadStrategy, SelectionSnapshot};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationValuePattern, UIA_ValuePatternId,
};

use crate::clipboard;
use crate::foreground::foreground_info;
use crate::read::{get_pattern, read_selection};

/// Capture the focused element and enough immutable context to reject stale
/// writes after focus or content drift.
pub(crate) fn capture(element: &IUIAutomationElement) -> Result<SelectionSnapshot, AdapterError> {
    let before = foreground_info()?;
    let is_password = unsafe { element.CurrentIsPassword() }
        .map(|v| v.as_bool())
        .unwrap_or(false);
    if is_password {
        return Err(AdapterError::SensitiveControl);
    }
    let enabled = unsafe { element.CurrentIsEnabled() }
        .map(|v| v.as_bool())
        .unwrap_or(true);
    if !enabled {
        return Err(AdapterError::UnsupportedControl);
    }

    let value_pattern = get_pattern::<IUIAutomationValuePattern>(element, UIA_ValuePatternId);
    let is_readonly = value_pattern
        .as_ref()
        .and_then(|v| unsafe { v.CurrentIsReadOnly() }.ok())
        .map(|v| v.as_bool())
        .unwrap_or(false);
    if is_readonly {
        return Err(AdapterError::UnsupportedControl);
    }

    let mut read = read_selection(element)?;
    if read.range_count > 1 {
        return Err(AdapterError::MultipleSelection);
    }
    if read.strategy == ReadStrategy::ClipboardFallback {
        read.text = clipboard::copy_selected_text()?;
    }
    if read.text.is_empty() {
        return Err(AdapterError::NoSelection);
    }

    let runtime_id = runtime_id(element)?;
    let after = foreground_info()?;
    if before.hwnd != after.hwnd {
        return Err(AdapterError::TargetChanged);
    }

    let full_text_hash = stable_text_hash(read.full_text.as_deref().unwrap_or(&read.text));
    let full_text_len = read.full_text.as_ref().map(|s| s.chars().count());
    Ok(SelectionSnapshot {
        hwnd: before.hwnd,
        runtime_id,
        process_name: before.process_name,
        selected_text: read.text,
        full_text_hash,
        selection_start: read.selection_start,
        full_text_len,
        is_readonly,
        is_password,
        read_strategy: read.strategy,
        captured_at: Instant::now(),
    })
}

/// Validate window, focused element identity, sensitivity and unchanged active
/// selection before a write. Returns the current focused element on success.
pub(crate) fn validate(
    expected: &SelectionSnapshot,
    element: &IUIAutomationElement,
) -> Result<(), AdapterError> {
    validate_identity(expected.hwnd, &expected.runtime_id, element)?;

    let mut current = read_selection(element)?;
    if current.strategy == ReadStrategy::ClipboardFallback {
        current.text = clipboard::copy_selected_text()?;
    }
    if current.text != expected.selected_text {
        return Err(AdapterError::TargetChanged);
    }
    let full_text = current.full_text.as_deref().unwrap_or(&current.text);
    if stable_text_hash(full_text) != expected.full_text_hash {
        return Err(AdapterError::TargetChanged);
    }
    Ok(())
}

/// Validate target identity without requiring the original selection to remain.
/// Used by undo after the write has intentionally collapsed the selection.
pub(crate) fn validate_identity(
    expected_hwnd: isize,
    expected_runtime_id: &[i32],
    element: &IUIAutomationElement,
) -> Result<(), AdapterError> {
    let current_window = foreground_info()?;
    if current_window.hwnd != expected_hwnd || runtime_id(element)? != expected_runtime_id {
        return Err(AdapterError::TargetChanged);
    }
    if unsafe { element.CurrentIsPassword() }
        .map(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(AdapterError::SensitiveControl);
    }
    Ok(())
}

/// Extract and own the integer UIA RuntimeId SAFEARRAY.
pub(crate) fn runtime_id(element: &IUIAutomationElement) -> Result<Vec<i32>, AdapterError> {
    // SAFETY: valid UIA element; caller owns the returned SAFEARRAY.
    let array = unsafe { element.GetRuntimeId() }
        .map_err(|e| AdapterError::Platform(format!("GetRuntimeId: {e}")))?;
    if array.is_null() {
        return Err(AdapterError::UnsupportedControl);
    }

    let result = (|| {
        // SAFEARRAY dimensions are one-based in these APIs.
        let lower = unsafe { SafeArrayGetLBound(array, 1) }
            .map_err(|e| AdapterError::Platform(format!("SafeArrayGetLBound: {e}")))?;
        let upper = unsafe { SafeArrayGetUBound(array, 1) }
            .map_err(|e| AdapterError::Platform(format!("SafeArrayGetUBound: {e}")))?;
        let mut result = Vec::with_capacity((upper - lower + 1).max(0) as usize);
        for index in lower..=upper {
            let mut value = 0i32;
            unsafe { SafeArrayGetElement(array, &index, (&mut value as *mut i32).cast()) }
                .map_err(|e| AdapterError::Platform(format!("SafeArrayGetElement: {e}")))?;
            result.push(value);
        }
        Ok(result)
    })();

    // SAFETY: balances ownership from GetRuntimeId.
    let _ = unsafe { SafeArrayDestroy(array) };
    result
}

/// Deterministic FNV-1a over UTF-8 bytes; suitable for drift detection (not
/// cryptographic or persisted authentication).
pub(crate) fn stable_text_hash(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_content_sensitive() {
        assert_eq!(stable_text_hash("中文"), stable_text_hash("中文"));
        assert_ne!(stable_text_hash("中文"), stable_text_hash("中文!"));
        assert_ne!(stable_text_hash("ab"), stable_text_hash("ba"));
    }
}
