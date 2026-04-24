use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Chat role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message sent to an LLM provider. Slice 1 only uses plain-text content;
/// multipart content (images, documents) lands in a later slice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
        }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
        }
    }
}

/// A tool-call the provider wants the agent to execute. Defined here for
/// forward compat with slice 3; slice 1 never populates it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationSettings {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
}

/// Minimal tool schema type — slice 1 always passes an empty slice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A single frame of a streaming response.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental assistant text.
    ContentDelta(String),
    /// Terminal event carrying the complete response (content, tool calls,
    /// usage). Producers must emit exactly one `Done` per stream.
    Done(LLMResponse),
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Generate a single non-streaming completion.
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse>;

    /// Generate a streaming completion. Default impl synthesizes a single
    /// `ContentDelta` + `Done` from `generate()` — override for true
    /// token-by-token streaming.
    fn generate_stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        Box::pin(async_stream::try_stream! {
            let response = self.generate(model, messages, tools, settings).await?;
            if let Some(ref content) = response.content {
                if !content.is_empty() {
                    yield StreamEvent::ContentDelta(content.clone());
                }
            }
            yield StreamEvent::Done(response);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;

    struct Constant(String);

    #[async_trait]
    impl LLMProvider for Constant {
        async fn generate(
            &self,
            _model: &str,
            _messages: &[ChatMessage],
            _tools: &[ToolSchema],
            _settings: &GenerationSettings,
        ) -> Result<LLMResponse> {
            Ok(LLMResponse {
                content: Some(self.0.clone()),
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn default_generate_stream_yields_delta_then_done() {
        let provider = Constant("hello".into());
        let messages = [ChatMessage::user("hi")];
        let tools: [ToolSchema; 0] = [];
        let settings = GenerationSettings::default();
        let stream = provider.generate_stream("m", &messages, &tools, &settings);
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 2);
        match &events[0] {
            Ok(StreamEvent::ContentDelta(s)) => assert_eq!(s, "hello"),
            other => panic!("expected ContentDelta, got {other:?}"),
        }
        match &events[1] {
            Ok(StreamEvent::Done(resp)) => {
                assert_eq!(resp.content.as_deref(), Some("hello"));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_content_skips_delta_but_emits_done() {
        struct Empty;
        #[async_trait]
        impl LLMProvider for Empty {
            async fn generate(
                &self,
                _model: &str,
                _messages: &[ChatMessage],
                _tools: &[ToolSchema],
                _settings: &GenerationSettings,
            ) -> Result<LLMResponse> {
                Ok(LLMResponse {
                    content: None,
                    tool_calls: Vec::new(),
                    usage: Usage::default(),
                })
            }
        }
        let provider = Empty;
        let messages = [ChatMessage::user("hi")];
        let tools: [ToolSchema; 0] = [];
        let settings = GenerationSettings::default();
        let stream = provider.generate_stream("m", &messages, &tools, &settings);
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Ok(StreamEvent::Done(_))));
    }
}
