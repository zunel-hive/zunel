//! LLM provider trait and implementations.

mod base;
mod build;
mod error;
mod openai_compat;
pub mod sse;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, StreamEvent,
    ToolCallRequest, ToolSchema, Usage,
};
pub use build::build_provider;
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
