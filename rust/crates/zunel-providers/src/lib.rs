//! LLM provider trait and implementations.

mod base;
mod build;
pub mod codex;
pub mod error;
mod openai_compat;
pub mod sse;
mod tool_call_accumulator;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, StreamEvent, ToolCallDelta,
    ToolCallRequest, ToolProgress, ToolSchema, Usage,
};
pub use build::build_provider;
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
pub use tool_call_accumulator::{ToolCallAccumulator, ToolCallAssemblyError};
