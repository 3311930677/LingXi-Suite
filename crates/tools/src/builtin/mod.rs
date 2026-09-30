//! Cross-platform built-in tools (ROADMAP 0.4).
//!
//! These have no OS dependencies and can be registered on any platform.
//! Tools that need network access (`web_fetch`, `translate`) are only
//! compiled with the `net` feature so the base crate stays dependency-free.

pub mod calc;
pub mod reminder;
pub mod time;

#[cfg(feature = "net")]
pub mod search;

#[cfg(feature = "net")]
pub mod fetch;
#[cfg(feature = "net")]
pub mod translate;
