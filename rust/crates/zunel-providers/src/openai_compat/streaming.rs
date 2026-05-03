//! Streaming `chat.completions` decoder for [`super::OpenAICompatProvider`].
//!
//! `stream_impl` wraps an `async_stream::try_stream!` block that:
//!  1. POSTs the streaming request,
//!  2. feeds bytes into [`SseBuffer`] to extract `data:` payloads,
//!  3. routes each payload through the OpenAI delta schema, and
//!  4. yields [`StreamEvent::ContentDelta`] / [`StreamEvent::ToolCallDelta`]
//!     incrementally, then [`StreamEvent::Done`] with the assembled
//!     [`LLMResponse`] (content + tool calls + usage).

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::base::{ChatMessage, GenerationSettings, LLMResponse, StreamEvent, ToolSchema};
use crate::error::{Error, Result};
use crate::sse::SseBuffer;
use crate::tool_call_accumulator::ToolCallAccumulator;

use super::wire::{RequestBody, WireUsage};
use super::OpenAICompatProvider;

#[derive(Serialize)]
pub(super) struct StreamRequestBody<'a> {
    model: &'a str,
    messages: Vec<super::wire::WireMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<super::wire::WireTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

impl<'a> StreamRequestBody<'a> {
    fn new(
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        settings: &GenerationSettings,
    ) -> Self {
        let inner = RequestBody::new(model, messages, tools, settings);
        Self {
            model: inner.model,
            messages: inner.messages,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
            temperature: inner.temperature,
            max_tokens: inner.max_tokens,
            tools: inner.tools,
            tool_choice: inner.tool_choice,
        }
    }
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    /// Carried through from the provider and forwarded to the agent
    /// runner via `LLMResponse.finish_reason`. "stop", "length",
    /// "tool_calls", "content_filter" are the documented values;
    /// anything else passes through unchanged.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    /// Tool call fragments. OpenAI disambiguates parallel calls by
    /// `index`; id + name generally arrive in the first chunk for an
    /// index and `arguments` stream across subsequent chunks.
    #[serde(default)]
    tool_calls: Vec<StreamDeltaToolCall>,
}

#[derive(Deserialize)]
struct StreamDeltaToolCall {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamDeltaFunction>,
}

#[derive(Deserialize)]
struct StreamDeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

impl OpenAICompatProvider {
    pub(crate) fn stream_impl<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        let client = self.client.clone();
        let url = format!("{}/chat/completions", self.api_base);
        let body = StreamRequestBody::new(model, messages, tools, settings);

        Box::pin(async_stream::try_stream! {
            let response = client.post(&url).json(&body).send().await?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                Err(Error::ProviderReturned { status: status.as_u16(), body: text })?;
                return;
            }

            let mut buffer = SseBuffer::new();
            let mut accumulated = String::new();
            let mut final_usage: Option<WireUsage> = None;
            let mut final_finish_reason: Option<String> = None;
            let mut tool_call_acc = ToolCallAccumulator::default();
            let mut stream = response.bytes_stream();

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Network)?;
                let events = buffer.feed(&chunk);
                for event in events {
                    match event {
                        None => {
                            tracing::debug!(
                                model = %model,
                                finish_reason = final_finish_reason.as_deref().unwrap_or("<none>"),
                                "openai-compat: stream done",
                            );
                            let tool_calls = tool_call_acc
                                .finalize()
                                .map_err(|e| Error::Parse(format!("tool_call reassembly: {e}")))?;
                            let response = LLMResponse {
                                content: if accumulated.is_empty() {
                                    None
                                } else {
                                    Some(accumulated.clone())
                                },
                                tool_calls,
                                usage: final_usage.take().unwrap_or_default().into(),
                                finish_reason: final_finish_reason.take(),
                            };
                            yield StreamEvent::Done(response);
                            return;
                        }
                        Some(payload) => {
                            let parsed: StreamChunk = serde_json::from_str(&payload)
                                .map_err(|e| Error::Parse(format!("chunk decode: {e}")))?;
                            for choice in parsed.choices {
                                if let Some(text) = choice.delta.content {
                                    if !text.is_empty() {
                                        accumulated.push_str(&text);
                                        yield StreamEvent::ContentDelta(text);
                                    }
                                }
                                for tc in choice.delta.tool_calls {
                                    let (name, arguments_fragment) = match tc.function {
                                        Some(f) => (f.name, f.arguments),
                                        None => (None, None),
                                    };
                                    let delta = StreamEvent::ToolCallDelta {
                                        index: tc.index,
                                        id: tc.id,
                                        name,
                                        arguments_fragment,
                                    };
                                    tool_call_acc.push(delta.clone());
                                    yield delta;
                                }
                                if let Some(reason) = choice.finish_reason {
                                    final_finish_reason = Some(reason);
                                }
                            }
                            if let Some(u) = parsed.usage {
                                final_usage = Some(u);
                            }
                        }
                    }
                }
            }
            tracing::debug!(
                model = %model,
                finish_reason = final_finish_reason.as_deref().unwrap_or("<none>"),
                "openai-compat: stream ended without [DONE]",
            );
            let tool_calls = tool_call_acc
                .finalize()
                .map_err(|e| Error::Parse(format!("tool_call reassembly: {e}")))?;
            let response = LLMResponse {
                content: if accumulated.is_empty() { None } else { Some(accumulated) },
                tool_calls,
                usage: final_usage.unwrap_or_default().into(),
                finish_reason: final_finish_reason,
            };
            yield StreamEvent::Done(response);
        })
    }
}
