//! LLM provider trait and implementations.

mod base;
mod error;
mod openai_compat;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolCallRequest, ToolSchema,
    Usage,
};
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
