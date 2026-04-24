use std::collections::BTreeMap;

use async_trait::async_trait;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};

use crate::base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolSchema, Usage,
};
use crate::error::{Error, Result};

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
        let body = RequestBody::new(model, messages, settings);
        let url = format!("{}/chat/completions", self.api_base);
        let response = self.client.post(&url).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::ProviderReturned {
                status: status.as_u16(),
                body,
            });
        }
        let parsed: ResponseBody = response
            .json()
            .await
            .map_err(|e| Error::Parse(format!("json decode: {e}")))?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Parse("response had no choices".into()))?;
        Ok(LLMResponse {
            content: choice.message.content,
            tool_calls: Vec::new(),
            usage: parsed.usage.unwrap_or_default().into(),
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
