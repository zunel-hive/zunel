//! LLM provider trait and implementations.

mod base;
mod error;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolCallRequest, ToolSchema,
    Usage,
};
pub use error::{Error, Result};
