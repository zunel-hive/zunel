pub mod schema;
pub mod stdio;
pub mod wrapper;

pub use schema::normalize_schema_for_openai;
pub use stdio::{McpToolDefinition, StdioMcpClient};
pub use wrapper::McpToolWrapper;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mcp io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mcp json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("mcp protocol error: {0}")]
    Protocol(String),
    #[error("mcp timeout after {0}s")]
    Timeout(u64),
}

pub type Result<T> = std::result::Result<T, Error>;
