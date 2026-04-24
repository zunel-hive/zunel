use std::sync::Arc;

use zunel_config::AgentDefaults;
use zunel_providers::{ChatMessage, GenerationSettings, LLMProvider};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
}

/// Minimal agent loop for slice 1. One user message in, one assistant message
/// out. No tools, no history, no context builder. Slice 2 adds sessions and
/// streaming; slice 3 adds tools.
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    defaults: AgentDefaults,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn LLMProvider>, defaults: AgentDefaults) -> Self {
        Self { provider, defaults }
    }

    /// Run a single user message through the provider and return the reply.
    pub async fn process_direct(&self, message: &str) -> Result<RunResult> {
        let settings = GenerationSettings {
            temperature: self.defaults.temperature,
            max_tokens: self.defaults.max_tokens,
            reasoning_effort: self.defaults.reasoning_effort.clone(),
        };
        let messages = vec![ChatMessage::user(message)];
        tracing::debug!(model = %self.defaults.model, "agent_loop: generating");
        let response = self
            .provider
            .generate(&self.defaults.model, &messages, &[], &settings)
            .await?;
        Ok(RunResult {
            content: response.content.unwrap_or_default(),
            tools_used: Vec::new(),
            messages,
        })
    }
}
