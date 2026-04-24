use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, StreamEvent, ToolSchema,
};
use crate::error::{Error, Result};
use crate::responses::{convert_messages, convert_tools, ResponsesStreamParser};
use crate::sse::SseBuffer;

pub const DEFAULT_CODEX_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.4";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_USER_AGENT: &str = "zunel (rust)";

const CODEX_LOGIN_HINT: &str =
    "Sign in with `codex login` using file-backed credentials, then retry.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: String,
}

#[async_trait]
pub trait CodexAuthProvider: Send + Sync {
    async fn load(&self) -> Result<CodexAuth>;
}

#[derive(Debug, Clone)]
pub struct FileCodexAuthProvider {
    codex_home: PathBuf,
}

impl FileCodexAuthProvider {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn from_env() -> Result<Self> {
        if let Ok(home) = std::env::var("CODEX_HOME") {
            return Ok(Self::new(PathBuf::from(home)));
        }
        let home = std::env::var("HOME").map_err(|_| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: HOME is not set. {CODEX_LOGIN_HINT}"
            ))
        })?;
        Ok(Self::new(PathBuf::from(home).join(".codex")))
    }

    fn auth_path(&self) -> PathBuf {
        self.codex_home.join("auth.json")
    }
}

pub struct CodexProvider {
    client: reqwest::Client,
    api_base: String,
    auth: Arc<dyn CodexAuthProvider>,
}

impl CodexProvider {
    pub fn new(api_base: Option<String>) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            api_base: api_base.unwrap_or_else(|| DEFAULT_CODEX_URL.to_string()),
            auth: Arc::new(FileCodexAuthProvider::from_env()?),
        })
    }

    pub fn with_auth(api_base: String, auth: Arc<dyn CodexAuthProvider>) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            api_base,
            auth,
        })
    }

    pub fn default_model(&self) -> &'static str {
        DEFAULT_CODEX_MODEL
    }

    fn request_body(
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<Value> {
        let converted = convert_messages(messages)?;
        let mut body = json!({
            "model": model,
            "store": false,
            "stream": true,
            "instructions": converted.instructions,
            "input": converted.input,
            "text": {"verbosity": "medium"},
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": prompt_cache_key(messages)?,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
        });
        if let Some(effort) = &settings.reasoning_effort {
            body["reasoning"] = json!({"effort": effort});
        }
        if !tools.is_empty() {
            body["tools"] = convert_tools(tools);
        }
        Ok(body)
    }
}

#[async_trait]
impl LLMProvider for CodexProvider {
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse> {
        let mut stream = self.generate_stream(model, messages, tools, settings);
        let mut final_response = None;
        use futures::StreamExt;
        while let Some(event) = stream.next().await {
            if let StreamEvent::Done(resp) = event? {
                final_response = Some(resp);
            }
        }
        final_response.ok_or_else(|| Error::Parse("codex stream ended without Done".into()))
    }

    fn generate_stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        Box::pin(async_stream::try_stream! {
            let token = self.auth.load().await?;
            let body = Self::request_body(model, messages, tools, settings)?;
            let response = self
                .client
                .post(&self.api_base)
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token.access_token))
                .header("chatgpt-account-id", token.account_id)
                .header("OpenAI-Beta", "responses=experimental")
                .header("originator", CODEX_ORIGINATOR)
                .header(reqwest::header::USER_AGENT, CODEX_USER_AGENT)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                Err(Error::ProviderReturned {
                    status: status.as_u16(),
                    body: friendly_error(status.as_u16(), &text),
                })?;
                return;
            }

            let mut sse = SseBuffer::new();
            let mut parser = ResponsesStreamParser::new();
            let mut saw_done = false;
            let mut stream = response.bytes_stream();
            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Network)?;
                for payload in sse.feed(&chunk) {
                    let Some(payload) = payload else {
                        if !saw_done {
                            for event in parser.finish()? {
                                yield event;
                            }
                        }
                        return;
                    };
                    let value: Value = serde_json::from_str(&payload)
                        .map_err(|e| Error::Parse(format!("codex event decode: {e}")))?;
                    for event in parser.accept(&value)? {
                        saw_done = saw_done || matches!(event, StreamEvent::Done(_));
                        yield event;
                    }
                }
            }
            if !saw_done {
                for event in parser.finish()? {
                    yield event;
                }
            }
        })
    }
}

#[async_trait]
impl CodexAuthProvider for FileCodexAuthProvider {
    async fn load(&self) -> Result<CodexAuth> {
        let path = self.auth_path();
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: failed to read {}: {e}. {CODEX_LOGIN_HINT}",
                path.display()
            ))
        })?;
        let value: Value = serde_json::from_str(&raw).map_err(|e| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: failed to parse {}: {e}. {CODEX_LOGIN_HINT}",
                path.display()
            ))
        })?;
        let access_token = value
            .pointer("/tokens/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                Error::Auth(format!(
                    "Codex OAuth credentials unavailable: auth.json does not contain an access token. {CODEX_LOGIN_HINT}"
                ))
            })?
            .to_string();
        let account_id = find_account_id(&value).ok_or_else(|| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: auth.json does not contain a ChatGPT account id. {CODEX_LOGIN_HINT}"
            ))
        })?;

        Ok(CodexAuth {
            access_token,
            account_id,
        })
    }
}

fn find_account_id(value: &Value) -> Option<String> {
    [
        "/account_id",
        "/chatgpt_account_id",
        "/account/id",
        "/profile/account_id",
        "/tokens/account_id",
    ]
    .iter()
    .find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn prompt_cache_key(messages: &[ChatMessage]) -> Result<String> {
    let raw = serde_json::to_string(messages)
        .map_err(|e| Error::Parse(format!("prompt cache key encode: {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

fn friendly_error(status: u16, raw: &str) -> String {
    match status {
        401 | 403 => format!(
            "HTTP {status}: Codex credentials were rejected. Re-run `codex login` and retry."
        ),
        429 => "ChatGPT usage quota exceeded or rate limit triggered. Please try again later."
            .to_string(),
        _ => format!(
            "HTTP {status}: {}",
            raw.chars().take(500).collect::<String>()
        ),
    }
}
