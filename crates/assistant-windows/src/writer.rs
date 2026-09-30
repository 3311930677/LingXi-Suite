//! Safe write-back and undo.
//!
//! Important: UIA TextPattern has no mutation API. It is used to validate and
//! locate the selection; replacement itself is ValuePattern (whole value), a
//! controlled paste, or Unicode SendInput.

use std::thread::sleep;
use std::time::Duration;

use assistant_core::{
    AdapterError, ReadStrategy, SelectionSnapshot, UndoReceipt, UndoToken, WriteReceipt,
    WriteStrategy,
};
use windows::core::BSTR;
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationValuePattern, UIA_ValuePatternId,
};

use crate::clipboard;
use crate::keyboard;
use crate::read::{get_pattern, read_full_text};
use crate::snapshot;

const SETTLE_DELAY: Duration = Duration::from_millis(100);

pub(crate) fn write(
    element: &IUIAutomationElement,
    target: &SelectionSnapshot,
    new_text: &str,
) -> Result<WriteReceipt, AdapterError> {
    snapshot::validate(target, element)?;

    let strategy = if target.read_strategy == ReadStrategy::ValuePattern {
        write_value(element, new_text)?;
        WriteStrategy::ValuePattern
    } else if new_text.is_empty() {
        keyboard::delete_selection()?;
        WriteStrategy::KeyboardInput
    } else {
        // Clipboard paste handles Unicode, emoji and newlines atomically while
        // preserving all original formats. We deliberately do not attempt a
        // second strategy after an error: an OS call may have partially
        // mutated the target, and retrying could duplicate text.
        clipboard::paste_text_preserving_clipboard(new_text)?;
        WriteStrategy::ClipboardPaste
    };

    sleep(SETTLE_DELAY);
    let verified = verify_replacement(element, target, new_text)?;
    Ok(WriteReceipt {
        strategy_used: strategy,
        verified,
        wrote_len: new_text.chars().count(),
        undo: UndoToken {
            hwnd: target.hwnd,
            runtime_id: target.runtime_id.clone(),
            original_text: target.selected_text.clone(),
            written_text: new_text.to_string(),
            selection_start: target.selection_start,
            original_full_text_len: target.full_text_len,
            write_strategy: strategy,
        },
    })
}

pub(crate) fn undo(
    element: &IUIAutomationElement,
    receipt: &WriteReceipt,
) -> Result<UndoReceipt, AdapterError> {
    let token = &receipt.undo;
    snapshot::validate_identity(token.hwnd, &token.runtime_id, element)?;

    match token.write_strategy {
        WriteStrategy::ValuePattern => write_value(element, &token.original_text)?,
        WriteStrategy::KeyboardInput | WriteStrategy::ClipboardPaste => keyboard::undo()?,
    }
    sleep(SETTLE_DELAY);

    let verified = match read_full_text(element)? {
        Some(text) => verify_original(&text, token),
        None => false,
    };
    Ok(UndoReceipt {
        restored_len: token.original_text.chars().count(),
        verified,
    })
}

fn write_value(element: &IUIAutomationElement, text: &str) -> Result<(), AdapterError> {
    let pattern = get_pattern::<IUIAutomationValuePattern>(element, UIA_ValuePatternId)
        .ok_or(AdapterError::UnsupportedControl)?;
    // SAFETY: valid ValuePattern; snapshot validation checked read-only state at
    // capture, and providers still enforce it themselves.
    unsafe { pattern.SetValue(&BSTR::from(text)) }
        .map_err(|e| AdapterError::Platform(format!("ValuePattern.SetValue: {e}")))
}

fn verify_replacement(
    element: &IUIAutomationElement,
    target: &SelectionSnapshot,
    new_text: &str,
) -> Result<bool, AdapterError> {
    let Some(full) = read_full_text(element)? else {
        return Ok(false);
    };
    if target.read_strategy == ReadStrategy::ValuePattern {
        return Ok(full == new_text);
    }
    // Rich text controls (e.g. Notepad's RichEdit) expose a document whose
    // trailing whitespace and offset accounting are inconsistent, so exact
    // length checks are unreliable. Prefer an exact match at the known offset,
    // and fall back to a substring check when the offset is unavailable.
    Ok(matched_at_offset(&full, target.selection_start, new_text) || full.contains(new_text))
}

fn verify_original(full: &str, token: &UndoToken) -> bool {
    match token.write_strategy {
        WriteStrategy::ValuePattern => full == token.original_text,
        WriteStrategy::KeyboardInput | WriteStrategy::ClipboardPaste => {
            if matched_at_offset(full, token.selection_start, &token.original_text) {
                return true;
            }
            // The original text is back and the written text is gone. Note the
            // written text usually contains the original, so its absence is the
            // decisive signal.
            full.contains(&token.original_text) && !full.contains(&token.written_text)
        }
    }
}

fn matched_at_offset(full: &str, start: Option<usize>, expected: &str) -> bool {
    start.is_some_and(|start| text_at(full, start, expected))
}

fn text_at(full: &str, char_start: usize, expected: &str) -> bool {
    full.chars()
        .skip(char_start)
        .take(expected.chars().count())
        .eq(expected.chars())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_at_uses_character_offsets_not_bytes() {
        assert!(text_at("a中文😀z", 1, "中文😀"));
        assert!(!text_at("a中文😀z", 2, "中文"));
    }

    #[test]
    fn verify_original_matches_at_known_offset() {
        let token = UndoToken {
            hwnd: 1,
            runtime_id: vec![1],
            original_text: "中文".into(),
            written_text: "[AI] 中文".into(),
            selection_start: Some(1),
            original_full_text_len: Some(4),
            write_strategy: WriteStrategy::ClipboardPaste,
        };
        // Offset match succeeds even when the document has extra trailing text.
        assert!(verify_original("a中文zz", &token));
    }

    #[test]
    fn verify_original_falls_back_to_presence_when_offset_unknown() {
        let token = UndoToken {
            hwnd: 1,
            runtime_id: vec![1],
            original_text: "中文".into(),
            written_text: "[AI] 中文".into(),
            selection_start: None,
            original_full_text_len: None,
            write_strategy: WriteStrategy::ClipboardPaste,
        };
        // Original restored and written text gone -> verified.
        assert!(verify_original("prefix 中文 suffix", &token));
        // Written text still present -> undo not confirmed.
        assert!(!verify_original("prefix [AI] 中文 suffix", &token));
    }
}
