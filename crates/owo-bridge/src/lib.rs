//! owo-agent 执行引擎的 HTTP/SSE 桥接客户端。
//!
//! LingXi 桌宠的「重环」：把需要文件读写、命令执行、多步规划的重任务
//! 委托给本机 `owo-agent serve`（默认 127.0.0.1:4096），拿回流式进度、
//! 审批请求与改动 diff。契约来源：`OwO/agent-sdk/clients/ts/openapi.json`
//! （v0.7，契约冻结；本 crate 只消费，不改引擎）。
//!
//! ```no_run
//! use owo_bridge::{OwoBridgeClient, TurnRequest};
//! let client = OwoBridgeClient::new("http://127.0.0.1:4096");
//! let health = client.health()?;
//! let session = client.create_session("D:/work", None, None)?;
//! client.turn_stream(&session.id, &TurnRequest::new("把 README 的错别字修一下"), |frame| {
//!     if let Some(event) = frame.json::<owo_bridge::TurnEvent>() {
//!         println!("{event:?}");
//!     }
//! })?;
//! # Ok::<(), owo_bridge::BridgeError>(())
//! ```

mod client;
mod sse;
mod types;

pub use client::{BridgeError, OwoBridgeClient};
pub use sse::{read_frames, SseFrame, SseParser};
pub use types::{
    CreateSessionRequest, FileDiff, HealthResponse, PermissionResponse, SessionInfo, TurnEvent,
    TurnRequest,
};

/// 默认服务端口（与 `start-agent-server.cmd` 一致）。
pub const DEFAULT_PORT: u16 = 4096;

/// 默认服务地址。
pub fn default_base_url() -> String {
    format!("http://127.0.0.1:{DEFAULT_PORT}")
}
