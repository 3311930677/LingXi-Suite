//! Inspect a focused UIA element and report which patterns it exposes.

use assistant_core::{AdapterError, CapabilityReport, ReadStrategy, WriteStrategy};
use windows::core::Interface;
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationValuePattern, UIA_TextPatternId,
    UIA_ValuePatternId, UIA_CONTROLTYPE_ID,
};

use crate::read::{read_selection, SelectionRead};

/// Full diagnostic result for the focused element, used by the `probe` tool.
pub struct FocusedProbe {
    pub name: String,
    pub class_name: String,
    pub control_type: String,
    pub is_password: bool,
    pub is_enabled: bool,
    pub is_readonly: bool,
    pub capability: CapabilityReport,
    /// The current selection read from the control (W2).
    pub selection: SelectionRead,
}

/// Inspect a focused element and build both a human-readable diagnostic and a
/// [`CapabilityReport`].
pub(crate) fn inspect(element: &IUIAutomationElement) -> Result<FocusedProbe, AdapterError> {
    // Basic properties. Missing values degrade to defaults rather than error,
    // because some controls do not expose every property.
    let name = unsafe { element.CurrentName() }
        .map(|b| b.to_string())
        .unwrap_or_default();
    let class_name = unsafe { element.CurrentClassName() }
        .map(|b| b.to_string())
        .unwrap_or_default();
    let is_password = unsafe { element.CurrentIsPassword() }
        .map(|b| b.as_bool())
        .unwrap_or(false);
    let is_enabled = unsafe { element.CurrentIsEnabled() }
        .map(|b| b.as_bool())
        .unwrap_or(true);
    let control_type = unsafe { element.CurrentControlType() }
        .map(control_type_name)
        .unwrap_or_else(|_| "Unknown".to_string());

    // Pattern availability. `GetCurrentPattern` returns the pattern object;
    // an unavailable pattern yields an error or a non-castable object, both of
    // which we treat as "not available".
    let has_text_pattern = unsafe { element.GetCurrentPattern(UIA_TextPatternId) }
        .ok()
        .and_then(|unk| unk.cast::<IUIAutomationTextPattern>().ok())
        .is_some();

    let value_pattern = unsafe { element.GetCurrentPattern(UIA_ValuePatternId) }
        .ok()
        .and_then(|unk| unk.cast::<IUIAutomationValuePattern>().ok());
    let has_value_pattern = value_pattern.is_some();

    let is_readonly = match &value_pattern {
        Some(vp) => unsafe { vp.CurrentIsReadOnly() }
            .map(|b| b.as_bool())
            .unwrap_or(false),
        None => false,
    };

    let recommended_read = if has_text_pattern {
        ReadStrategy::TextPattern
    } else if has_value_pattern {
        ReadStrategy::ValuePattern
    } else {
        ReadStrategy::ClipboardFallback
    };

    // UIA TextPattern is read-only: it can inspect/select ranges but cannot
    // insert or replace text. Rich text controls therefore use a controlled
    // paste while ValuePattern-only controls can replace their whole value.
    let recommended_write = if has_value_pattern && !has_text_pattern && !is_readonly {
        WriteStrategy::ValuePattern
    } else if has_text_pattern {
        WriteStrategy::ClipboardPaste
    } else {
        WriteStrategy::KeyboardInput
    };

    let capability = CapabilityReport {
        has_text_pattern,
        has_value_pattern,
        is_readonly,
        is_password,
        recommended_read,
        recommended_write,
    };

    // Never ask a password provider for its current value. Some providers may
    // expose ValuePattern despite being marked sensitive.
    let selection = if is_password {
        SelectionRead {
            text: String::new(),
            strategy: recommended_read,
            truncated: false,
            range_count: 0,
            selection_start: None,
            full_text: None,
        }
    } else {
        read_selection(element)?
    };

    Ok(FocusedProbe {
        name,
        class_name,
        control_type,
        is_password,
        is_enabled,
        is_readonly,
        capability,
        selection,
    })
}

/// Map a UIA control-type id to a short readable name. Unknown ids are printed
/// with their numeric value so new controls can still be identified on a real
/// machine.
fn control_type_name(ct: UIA_CONTROLTYPE_ID) -> String {
    let label = match ct.0 {
        50000 => "Button",
        50001 => "Calendar",
        50002 => "CheckBox",
        50003 => "ComboBox",
        50004 => "Edit",
        50005 => "Hyperlink",
        50006 => "Image",
        50007 => "ListItem",
        50008 => "List",
        50009 => "Menu",
        50010 => "MenuBar",
        50011 => "MenuItem",
        50018 => "Tab",
        50019 => "TabItem",
        50020 => "Text",
        50021 => "ToolBar",
        50025 => "Custom",
        50026 => "Group",
        50030 => "Document",
        50032 => "Window",
        50033 => "Pane",
        50036 => "Table",
        other => return format!("Unmapped({other})"),
    };
    label.to_string()
}
