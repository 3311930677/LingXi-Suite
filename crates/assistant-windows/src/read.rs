//! Reading the current selection from a focused control.
//!
//! UIA read priority:
//! 1. `TextPattern.GetSelection` — the exact selected range.
//! 2. `ValuePattern.CurrentValue` — the complete value for simple controls.
//! 3. Clipboard fallback — performed by `snapshot` because it also needs
//!    keyboard input and transactional clipboard preservation.

use assistant_core::{AdapterError, ReadStrategy};
use windows::core::Interface;
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextRange,
    IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    UIA_TextPatternId, UIA_ValuePatternId, UIA_PATTERN_ID,
};

use crate::error::platform;

/// Hard limit for document diagnostics and verification.
const MAX_READ_CHARS: i32 = 1_000_000;

/// Text read from the active control.
#[derive(Debug, Clone)]
pub struct SelectionRead {
    pub text: String,
    pub strategy: ReadStrategy,
    pub truncated: bool,
    /// Number of disjoint ranges returned by TextPattern.
    pub range_count: usize,
    /// Character offset in the full document, when TextPattern can expose it.
    pub selection_start: Option<usize>,
    /// Complete readable document/value, capped at [`MAX_READ_CHARS`].
    pub full_text: Option<String>,
}

/// Apply the UIA part of the read degrade chain.
pub(crate) fn read_selection(
    element: &IUIAutomationElement,
) -> Result<SelectionRead, AdapterError> {
    if let Some(tp) = get_pattern::<IUIAutomationTextPattern>(element, UIA_TextPatternId) {
        return read_text_pattern(&tp);
    }

    if let Some(vp) = get_pattern::<IUIAutomationValuePattern>(element, UIA_ValuePatternId) {
        // SAFETY: FFI call on a valid pattern object.
        let value = unsafe { vp.CurrentValue() }
            .map_err(|e| platform("ValuePattern.CurrentValue", e))?
            .to_string();
        let truncated = value.chars().count() > MAX_READ_CHARS as usize;
        let value = cap(value);
        return Ok(SelectionRead {
            text: value.clone(),
            strategy: ReadStrategy::ValuePattern,
            truncated,
            range_count: 1,
            selection_start: Some(0),
            full_text: Some(value),
        });
    }

    Ok(SelectionRead {
        text: String::new(),
        strategy: ReadStrategy::ClipboardFallback,
        truncated: false,
        range_count: 0,
        selection_start: None,
        full_text: None,
    })
}

fn read_text_pattern(tp: &IUIAutomationTextPattern) -> Result<SelectionRead, AdapterError> {
    // SAFETY: FFI call; returned range array is owned by the wrapper.
    let ranges =
        unsafe { tp.GetSelection() }.map_err(|e| platform("TextPattern.GetSelection", e))?;
    // SAFETY: valid COM range array.
    let len = unsafe { ranges.Length() }.map_err(|e| platform("TextRangeArray.Length", e))?;

    let mut selected = String::new();
    let mut first_range: Option<IUIAutomationTextRange> = None;
    for i in 0..len {
        // SAFETY: index is within [0, len).
        let range = unsafe { ranges.GetElement(i) }
            .map_err(|e| platform("TextRangeArray.GetElement", e))?;
        if first_range.is_none() {
            first_range = Some(range.clone());
        }
        selected.push_str(&range_text(&range)?);
    }

    // SAFETY: valid pattern object.
    let document =
        unsafe { tp.DocumentRange() }.map_err(|e| platform("TextPattern.DocumentRange", e))?;
    let full_raw = range_text(&document)?;
    let truncated = full_raw.chars().count() > MAX_READ_CHARS as usize;
    let full_text = cap(full_raw);
    let selection_start = match first_range {
        Some(ref range) if len == 1 => selection_start(&document, range).ok(),
        _ => None,
    };

    Ok(SelectionRead {
        text: cap(selected),
        strategy: ReadStrategy::TextPattern,
        truncated,
        range_count: len as usize,
        selection_start,
        full_text: Some(full_text),
    })
}

/// Re-read only the complete document/value for post-write verification.
pub(crate) fn read_full_text(
    element: &IUIAutomationElement,
) -> Result<Option<String>, AdapterError> {
    if let Some(tp) = get_pattern::<IUIAutomationTextPattern>(element, UIA_TextPatternId) {
        // SAFETY: valid pattern object.
        let range =
            unsafe { tp.DocumentRange() }.map_err(|e| platform("TextPattern.DocumentRange", e))?;
        return range_text(&range).map(Some);
    }
    if let Some(vp) = get_pattern::<IUIAutomationValuePattern>(element, UIA_ValuePatternId) {
        // SAFETY: valid pattern object.
        let value =
            unsafe { vp.CurrentValue() }.map_err(|e| platform("ValuePattern.CurrentValue", e))?;
        return Ok(Some(cap(value.to_string())));
    }
    Ok(None)
}

/// Obtain a concrete UIA pattern or `None` when unsupported.
pub(crate) fn get_pattern<T: Interface>(
    element: &IUIAutomationElement,
    id: UIA_PATTERN_ID,
) -> Option<T> {
    // SAFETY: an unavailable pattern yields Err or a non-castable object.
    unsafe { element.GetCurrentPattern(id) }
        .ok()
        .and_then(|unk| unk.cast::<T>().ok())
}

fn range_text(range: &IUIAutomationTextRange) -> Result<String, AdapterError> {
    // SAFETY: FFI call on a valid text range.
    unsafe { range.GetText(MAX_READ_CHARS) }
        .map(|s| s.to_string())
        .map_err(|e| platform("TextRange.GetText", e))
}

fn selection_start(
    document: &IUIAutomationTextRange,
    selection: &IUIAutomationTextRange,
) -> Result<usize, AdapterError> {
    // Clone the document and move its end to the selection start. The text in
    // that range is exactly the prefix before the selected range.
    // SAFETY: valid UIA ranges from the same text provider.
    let prefix = unsafe { document.Clone() }.map_err(|e| platform("TextRange.Clone", e))?;
    // SAFETY: ranges belong to the same provider.
    unsafe {
        prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selection,
            TextPatternRangeEndpoint_Start,
        )
    }
    .map_err(|e| platform("TextRange.MoveEndpointByRange", e))?;
    Ok(range_text(&prefix)?.chars().count())
}

fn cap(s: String) -> String {
    if s.chars().count() > MAX_READ_CHARS as usize {
        s.chars().take(MAX_READ_CHARS as usize).collect()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_preserves_unicode_boundaries() {
        let source = "好".repeat(MAX_READ_CHARS as usize + 1);
        let capped = cap(source);
        assert_eq!(capped.chars().count(), MAX_READ_CHARS as usize);
        assert!(capped.is_char_boundary(capped.len()));
    }
}
