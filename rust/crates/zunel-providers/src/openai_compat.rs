use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, StreamEvent, ToolSchema, Usage,
};
use crate::error::{Error, Result};
use crate::sse::SseBuffer;

/// Provider hitting any OpenAI `chat.completions`-compatible endpoint.
pub struct OpenAICompatProvider {
    client: Client,
    api_base: String,
}

impl OpenAICompatProvider {
    pub fn new(
        api_key: String,
        api_base: String,
        extra_headers: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        let bearer = format!("Bearer {api_key}");
        let mut auth = header::HeaderValue::from_str(&bearer)
            .map_err(|e| Error::Config(format!("invalid api key: {e}")))?;
        auth.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        for (k, v) in extra_headers {
            let name = header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| Error::Config(format!("bad extra header name {k}: {e}")))?;
            let value = header::HeaderValue::from_str(&v)
                .map_err(|e| Error::Config(format!("bad extra header value for {k}: {e}")))?;
            headers.insert(name, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(Error::Network)?;
        Ok(Self {
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        _tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse> {
        const MAX_ATTEMPTS: u32 = 2;
        const MAX_WAIT: Duration = Duration::from_secs(5);

        let body = RequestBody::new(model, messages, settings);
        let url = format!("{}/chat/completions", self.api_base);

        let mut last_retry_after: Option<Duration> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            let response = self.client.post(&url).json(&body).send().await?;
            let status = response.status();

            if status.is_success() {
                let parsed: ResponseBody = response
                    .json()
                    .await
                    .map_err(|e| Error::Parse(format!("json decode: {e}")))?;
                let choice = parsed
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::Parse("response had no choices".into()))?;
                return Ok(LLMResponse {
                    content: choice.message.content,
                    tool_calls: Vec::new(),
                    usage: parsed.usage.unwrap_or_default().into(),
                });
            }

            if status.as_u16() == 429 && attempt < MAX_ATTEMPTS {
                let retry = parse_retry_after(response.headers())
                    .unwrap_or(Duration::from_millis(500))
                    .min(MAX_WAIT);
                last_retry_after = Some(retry);
                tracing::warn!(
                    attempt = attempt,
                    retry_after_ms = retry.as_millis() as u64,
                    "openai-compat: 429, retrying"
                );
                sleep(retry).await;
                continue;
            }

            if status.as_u16() == 429 {
                return Err(Error::RateLimited {
                    retry_after: last_retry_after,
                });
            }

            let text = response.text().await.unwrap_or_default();
            return Err(Error::ProviderReturned {
                status: status.as_u16(),
                body: text,
            });
        }
        unreachable!("loop always returns")
    }

    fn generate_stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        self.stream_impl(model, messages, settings)
    }
}

fn parse_retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let v = headers.get(header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = v.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    None
}

#[derive(Serialize)]
struct StreamRequestBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

impl<'a> StreamRequestBody<'a> {
    fn new(model: &'a str, messages: &'a [ChatMessage], settings: &GenerationSettings) -> Self {
        let inner = RequestBody::new(model, messages, settings);
        Self {
            model: inner.model,
            messages: inner.messages,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
            temperature: inner.temperature,
            max_tokens: inner.max_tokens,
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
    /// Carried through from the provider and logged at `debug` level on
    /// the terminal chunk so operators can see `"stop"`, `"length"`,
    /// `"tool_calls"`, or `"content_filter"` in traces. Slice 3's tool
    /// loop will read this to route control flow; until then it's an
    /// observability signal only.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

impl OpenAICompatProvider {
    pub(crate) fn stream_impl<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        let client = self.client.clone();
        let url = format!("{}/chat/completions", self.api_base);
        let body = StreamRequestBody::new(model, messages, settings);

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
                            let response = LLMResponse {
                                content: if accumulated.is_empty() {
                                    None
                                } else {
                                    Some(accumulated.clone())
                                },
                                tool_calls: Vec::new(),
                                usage: final_usage.take().unwrap_or_default().into(),
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
            let response = LLMResponse {
                content: if accumulated.is_empty() { None } else { Some(accumulated) },
                tool_calls: Vec::new(),
                usage: final_usage.unwrap_or_default().into(),
            };
            yield StreamEvent::Done(response);
        })
    }
}

#[derive(Serialize)]
struct RequestBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

impl<'a> RequestBody<'a> {
    fn new(model: &'a str, messages: &'a [ChatMessage], settings: &GenerationSettings) -> Self {
        let wire = messages
            .iter()
            .map(|m| WireMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                },
                content: &m.content,
            })
            .collect();
        Self {
            model,
            messages: wire,
            temperature: settings.temperature,
            max_tokens: settings.max_tokens,
        }
    }
}

#[derive(Deserialize)]
struct ResponseBody {
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    cached_tokens: u32,
}

impl From<WireUsage> for Usage {
    fn from(value: WireUsage) -> Self {
        Usage {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            cached_tokens: value.cached_tokens,
        }
    }
}
