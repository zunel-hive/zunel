use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use zunel_config::AgentDefaults;
use zunel_providers::{ChatMessage, GenerationSettings, LLMProvider, Role, StreamEvent};

use crate::error::Result;
use crate::session::{ChatRole, Session, SessionManager};

#[derive(Debug, Clone)]
pub struct RunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
}

/// Agent loop. Slice 1 shipped the one-shot, stateless `process_direct`.
/// Slice 2 adds `process_streamed` which uses a `SessionManager` for
/// persistent conversation history and streams deltas to the caller.
///
/// Concurrency note: `SessionManager` uses atomic temp-file-+-rename
/// writes and is safe for concurrent reads, but two simultaneous writes
/// to the same session will race on last-writer-wins semantics. Slice 2
/// expects single-turn-at-a-time access (the REPL is inherently
/// sequential); proper per-session locking arrives in slice 5 with the
/// gateway, using `fd-lock` to match Python's `filelock` behavior.
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    defaults: AgentDefaults,
    sessions: Option<Arc<SessionManager>>,
}

impl AgentLoop {
    /// Slice 1 constructor — stateless, no session persistence.
    pub fn new(provider: Arc<dyn LLMProvider>, defaults: AgentDefaults) -> Self {
        Self {
            provider,
            defaults,
            sessions: None,
        }
    }

    /// Slice 2 constructor — sessions persist to `<workspace>/sessions/`.
    pub fn with_sessions(
        provider: Arc<dyn LLMProvider>,
        defaults: AgentDefaults,
        sessions: SessionManager,
    ) -> Self {
        Self {
            provider,
            defaults,
            sessions: Some(Arc::new(sessions)),
        }
    }

    fn settings(&self) -> GenerationSettings {
        GenerationSettings {
            temperature: self.defaults.temperature,
            max_tokens: self.defaults.max_tokens,
            reasoning_effort: self.defaults.reasoning_effort.clone(),
        }
    }

    /// Stateless one-shot. Retained for slice 1 callers.
    pub async fn process_direct(&self, message: &str) -> Result<RunResult> {
        let settings = self.settings();
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

    /// Streaming turn with session persistence. Feeds the accumulated
    /// conversation to the provider, emits deltas via `sink`, and persists
    /// the user + assistant messages after the stream ends.
    ///
    /// `sink` may be dropped early by the caller (e.g. user hit Ctrl+C);
    /// the loop tolerates that and still completes the turn server-side.
    pub async fn process_streamed(
        &self,
        session_key: &str,
        message: &str,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<RunResult> {
        let sessions = self
            .sessions
            .as_ref()
            .expect("process_streamed requires with_sessions()");
        let mut session = sessions
            .load(session_key)?
            .unwrap_or_else(|| Session::new(session_key));

        session.add_message(ChatRole::User, message);
        let history = session.get_history(500);
        let chat_messages = history_to_chat_messages(&history);

        let settings = self.settings();
        tracing::debug!(
            model = %self.defaults.model,
            history_len = chat_messages.len(),
            "agent_loop: streaming",
        );

        let mut stream =
            self.provider
                .generate_stream(&self.defaults.model, &chat_messages, &[], &settings);

        let mut accumulated = String::new();
        let mut final_content: Option<String> = None;

        while let Some(event) = stream.next().await {
            let event = event?;
            match &event {
                StreamEvent::ContentDelta(delta) => accumulated.push_str(delta),
                StreamEvent::Done(resp) => {
                    final_content =
                        Some(resp.content.clone().unwrap_or_else(|| accumulated.clone()));
                }
            }
            // Best-effort: if the sink is dropped, keep consuming the
            // stream so the HTTP connection isn't hung.
            let _ = sink.send(event).await;
        }
        drop(stream);

        let content = final_content.unwrap_or(accumulated);
        session.add_message(ChatRole::Assistant, &content);
        sessions.save(&session)?;

        Ok(RunResult {
            content,
            tools_used: Vec::new(),
            messages: chat_messages,
        })
    }
}

/// Convert persisted `Value` messages (from Session::get_history) into
/// provider-ready `ChatMessage`s. Slice 2 only knows about user/assistant/
/// system; tool messages land in slice 3.
fn history_to_chat_messages(history: &[Value]) -> Vec<ChatMessage> {
    history
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str)?;
            let content = m.get("content").and_then(Value::as_str)?;
            let role_enum = match role {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => return None,
            };
            Some(ChatMessage {
                role: role_enum,
                content: content.to_string(),
                tool_call_id: None,
            })
        })
        .collect()
}
