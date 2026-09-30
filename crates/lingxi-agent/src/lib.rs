//! Agent engine: a ReAct-style loop that plans, calls tools, observes
//! results, and replies — all platform-agnostic.
//!
//! The engine depends only on `lingxi-tools` for the `Tool` trait. Model
//! interaction is abstracted behind [`AgentBackend`], which can be a cloud
//! function-calling client or a local ReAct prompt-based parser.

pub mod action;
pub mod backend;
pub mod engine;
pub mod error;
pub mod session;

pub use action::{AgentAction, ToolCall};
pub use backend::AgentBackend;
pub use engine::{AgentEngine, AgentRunReport};
pub use error::AgentError;
pub use session::{Message, Role, Session};
