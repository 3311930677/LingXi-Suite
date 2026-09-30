//! Platform-agnostic domain types and the adapter interface for the
//! cross-app input assistant.
//!
//! Concrete platform crates (e.g. `assistant-windows`) implement
//! [`InputAdapter`]. Core logic depends only on this trait, so it can be
//! exercised with a mock adapter in tests without touching a real desktop.

use std::time::Instant;

pub mod diff;
pub mod transform;

pub use diff::{diff_chars, diff_stats, DiffOp, DiffStats};
pub use transform::{transformer_by_name, Transformer};

/// Errors that any platform adapter may return.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// No UI element currently has keyboard focus.
    #[error("no focused UI element")]
    NoFocusedElement,
    /// The focused control exposes no usable read/write channel.
    #[error("focused control is not supported")]
    UnsupportedControl,
    /// The target changed between capture and write-back (focus drift).
    #[error("target changed since capture (focus drift)")]
    TargetChanged,
    /// The control is a password field or otherwise contains sensitive input.
    #[error("refusing to read or write a password/sensitive control")]
    SensitiveControl,
    /// The control has no non-empty text selection.
    #[error("no text is selected")]
    NoSelection,
    /// Multiple disjoint ranges are selected; safe replacement is ambiguous.
    #[error("multiple text ranges are selected")]
    MultipleSelection,
    /// A write completed but could not be confirmed by reading the target.
    #[error("write-back verification failed")]
    VerificationFailed,
    /// A platform / OS call failed.
    #[error("platform call failed: {0}")]
    Platform(String),
}

/// Which channel was used (or is recommended) to read the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStrategy {
    /// Read via UIA `TextPattern` (richest, supports real selections).
    TextPattern,
    /// Read via UIA `ValuePattern` (whole-value only).
    ValuePattern,
    /// Fall back to simulating Ctrl+C and reading the clipboard.
    ClipboardFallback,
}

/// Which channel was used (or is recommended) to write text back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStrategy {
    /// Write a simple control's complete value via `ValuePattern::SetValue`.
    ValuePattern,
    /// Replace the active selection by sending Unicode keyboard input.
    KeyboardInput,
    /// Controlled clipboard: preserve all formats, paste, then restore.
    ClipboardPaste,
}

/// Capabilities detected on the currently focused control.
#[derive(Debug, Clone)]
pub struct CapabilityReport {
    pub has_text_pattern: bool,
    pub has_value_pattern: bool,
    pub is_readonly: bool,
    pub is_password: bool,
    pub recommended_read: ReadStrategy,
    pub recommended_write: WriteStrategy,
}

/// A snapshot of the target taken at capture time.
///
/// Used before write-back to detect focus drift and avoid writing into the
/// wrong place (see milestone W4).
#[derive(Debug, Clone)]
pub struct SelectionSnapshot {
    /// Foreground window handle at capture time.
    pub hwnd: isize,
    /// UIA RuntimeId of the focused element.
    pub runtime_id: Vec<i32>,
    /// Process image name, e.g. "notepad.exe".
    pub process_name: String,
    /// The originally selected text.
    pub selected_text: String,
    /// Deterministic hash of the complete readable value at capture time.
    pub full_text_hash: u64,
    /// Character offset of the selected range in the document, when UIA can
    /// determine it. Used to verify selection replacement after writing.
    pub selection_start: Option<usize>,
    /// Character length of the complete readable value at capture time.
    pub full_text_len: Option<usize>,
    pub is_readonly: bool,
    pub is_password: bool,
    /// Which channel actually produced the selection.
    pub read_strategy: ReadStrategy,
    /// When the snapshot was taken.
    pub captured_at: Instant,
}

/// Opaque information required to safely undo one completed write.
#[derive(Debug, Clone)]
pub struct UndoToken {
    pub hwnd: isize,
    pub runtime_id: Vec<i32>,
    pub original_text: String,
    pub written_text: String,
    pub selection_start: Option<usize>,
    pub original_full_text_len: Option<usize>,
    pub write_strategy: WriteStrategy,
}

/// Result of a write-back attempt.
#[derive(Debug, Clone)]
pub struct WriteReceipt {
    pub strategy_used: WriteStrategy,
    /// Whether a re-read after writing matched what we intended to write.
    pub verified: bool,
    pub wrote_len: usize,
    pub undo: UndoToken,
}

/// Result of restoring a previous write.
#[derive(Debug, Clone)]
pub struct UndoReceipt {
    pub restored_len: usize,
    pub verified: bool,
}

/// Platform-agnostic adapter interface implemented by each platform crate.
pub trait InputAdapter {
    /// Probe which capabilities the focused control supports.
    fn probe_capability(&self) -> Result<CapabilityReport, AdapterError>;

    /// Capture the current selection together with a context snapshot.
    fn capture_selection(&self) -> Result<SelectionSnapshot, AdapterError>;

    /// Write `new_text` back to the original control, verifying `target` first.
    fn write_back(
        &self,
        target: &SelectionSnapshot,
        new_text: &str,
    ) -> Result<WriteReceipt, AdapterError>;

    /// Restore the text represented by a successful write receipt.
    fn undo(&self, receipt: &WriteReceipt) -> Result<UndoReceipt, AdapterError>;
}

/// Result of one capture-transform-write pipeline execution.
#[derive(Debug, Clone)]
pub struct TransformOutcome {
    pub original_text: String,
    pub transformed_text: String,
    /// Character-level diff from original to transformed text.
    pub diff: Vec<DiffOp>,
    pub receipt: WriteReceipt,
}

impl TransformOutcome {
    /// Insertion/deletion counts of this transformation.
    pub fn stats(&self) -> DiffStats {
        diff_stats(&self.diff)
    }
}

/// Platform-independent orchestration for the main product flow. Model-backed
/// code can supply any transformation closure; platform details remain behind
/// [`InputAdapter`] and can be tested with a mock.
pub fn transform_selection<A, F>(
    adapter: &A,
    transform: F,
) -> Result<TransformOutcome, AdapterError>
where
    A: InputAdapter,
    F: FnOnce(&str) -> String,
{
    let snapshot = adapter.capture_selection()?;
    let transformed_text = transform(&snapshot.selected_text);
    let diff = diff_chars(&snapshot.selected_text, &transformed_text);
    let receipt = adapter.write_back(&snapshot, &transformed_text)?;
    Ok(TransformOutcome {
        original_text: snapshot.selected_text,
        transformed_text,
        diff,
        receipt,
    })
}

/// Same pipeline driven by a [`Transformer`] instead of a closure.
pub fn transform_selection_with<A: InputAdapter>(
    adapter: &A,
    transformer: &dyn Transformer,
) -> Result<TransformOutcome, AdapterError> {
    transform_selection(adapter, |text| transformer.transform(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockAdapter {
        value: RefCell<String>,
    }

    impl MockAdapter {
        fn new(value: &str) -> Self {
            Self {
                value: RefCell::new(value.to_string()),
            }
        }
    }

    impl InputAdapter for MockAdapter {
        fn probe_capability(&self) -> Result<CapabilityReport, AdapterError> {
            Ok(CapabilityReport {
                has_text_pattern: true,
                has_value_pattern: false,
                is_readonly: false,
                is_password: false,
                recommended_read: ReadStrategy::TextPattern,
                recommended_write: WriteStrategy::ClipboardPaste,
            })
        }

        fn capture_selection(&self) -> Result<SelectionSnapshot, AdapterError> {
            let selected_text = self.value.borrow().clone();
            Ok(SelectionSnapshot {
                hwnd: 1,
                runtime_id: vec![42],
                process_name: "mock.exe".into(),
                full_text_hash: 0,
                selection_start: Some(0),
                full_text_len: Some(selected_text.chars().count()),
                selected_text,
                is_readonly: false,
                is_password: false,
                read_strategy: ReadStrategy::TextPattern,
                captured_at: Instant::now(),
            })
        }

        fn write_back(
            &self,
            target: &SelectionSnapshot,
            new_text: &str,
        ) -> Result<WriteReceipt, AdapterError> {
            if *self.value.borrow() != target.selected_text {
                return Err(AdapterError::TargetChanged);
            }
            *self.value.borrow_mut() = new_text.to_string();
            Ok(WriteReceipt {
                strategy_used: WriteStrategy::ClipboardPaste,
                verified: true,
                wrote_len: new_text.chars().count(),
                undo: UndoToken {
                    hwnd: target.hwnd,
                    runtime_id: target.runtime_id.clone(),
                    original_text: target.selected_text.clone(),
                    written_text: new_text.to_string(),
                    selection_start: target.selection_start,
                    original_full_text_len: target.full_text_len,
                    write_strategy: WriteStrategy::ClipboardPaste,
                },
            })
        }

        fn undo(&self, receipt: &WriteReceipt) -> Result<UndoReceipt, AdapterError> {
            if *self.value.borrow() != receipt.undo.written_text {
                return Err(AdapterError::TargetChanged);
            }
            *self.value.borrow_mut() = receipt.undo.original_text.clone();
            Ok(UndoReceipt {
                restored_len: receipt.undo.original_text.chars().count(),
                verified: true,
            })
        }
    }

    #[test]
    fn transform_and_undo_pipeline_is_platform_independent() {
        let adapter = MockAdapter::new("中文😀");
        let outcome = transform_selection(&adapter, |text| format!("[AI] {text}"))
            .expect("transform should succeed");
        assert_eq!(&*adapter.value.borrow(), "[AI] 中文😀");
        assert!(outcome.receipt.verified);

        let undo = adapter.undo(&outcome.receipt).expect("undo should succeed");
        assert!(undo.verified);
        assert_eq!(&*adapter.value.borrow(), "中文😀");
    }

    #[test]
    fn mock_rejects_content_drift_before_write() {
        let adapter = MockAdapter::new("before");
        let snapshot = adapter.capture_selection().unwrap();
        *adapter.value.borrow_mut() = "changed".into();
        assert!(matches!(
            adapter.write_back(&snapshot, "after"),
            Err(AdapterError::TargetChanged)
        ));
    }
}
