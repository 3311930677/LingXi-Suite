//! Error types for the agent engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("模型后端错误: {0}")]
    Backend(String),

    #[error("工具执行错误: {0}")]
    Tool(String),

    #[error("达到最大步数限制 ({0} 步)")]
    MaxStepsExceeded(usize),

    #[error("会话已关闭")]
    SessionClosed,

    #[error("序列化/反序列化错误: {0}")]
    Serde(#[from] serde_json::Error),
}
