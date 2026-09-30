//! Error helpers for converting windows-rs errors into [`AdapterError`].

use assistant_core::AdapterError;

/// Wrap a failing platform call into [`AdapterError::Platform`] with context.
pub(crate) fn platform<E: std::fmt::Display>(context: &str, err: E) -> AdapterError {
    AdapterError::Platform(format!("{context}: {err}"))
}
