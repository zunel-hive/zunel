use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;
use zunel_config::AgentDefaults;
use zunel_providers::{ChatMessage, GenerationSettings, LLMProvider, Role, StreamEvent};

use crate::approval::{AllowAllApprovalHandler, ApprovalHandler, ApprovalScope};
use crate::error::Result;
use crate::runner::{AgentRunSpec, AgentRunner};
use crate::session::{ChatRole, Session, SessionManager};
use crate::trim::chat_message_to_value;
use zunel_tools::ToolRegistry;

/// Maximum number of prior messages replayed to the provider per turn.
/// Matches Python's `AgentLoop` history cap in `zunel/agent/loop.py`.
/// Older messages beyond this window are retained on disk (via
/// `Session`) but are not sent to the LLM; slice 3's context builder
/// replaces this fixed window with a token-budget-aware trimmer.
const HISTORY_LIMIT: usize = 500;

/// Outcome of a single agent turn.
///
/// - `content`: the assistant text the provider produced (possibly empty
///   if the model returned no text, e.g. a pure tool-call turn in a
///   later slice).
/// - `tools_used`: names of tools the agent invoked during the turn.
///   Always empty in slice 2 — the tool loop lands in slice 3.
/// - `messages`: the ordered message history that was sent to the
///   provider for this turn (user-only for `process_direct`, the
///   truncated session history for `process_streamed`).
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
    tools: ToolRegistry,
    approval: Arc<dyn ApprovalHandler>,
    approval_required: bool,
    approval_scope: ApprovalScope,
    workspace: std::path::PathBuf,
}

impl AgentLoop {
    /// Slice 1 constructor — stateless, no session persistence.
    pub fn new(provider: Arc<dyn LLMProvider>, defaults: AgentDefaults) -> Self {
        Self {
            provider,
            defaults,
            sessions: None,
            tools: ToolRegistry::new(),
            approval: Arc::new(AllowAllApprovalHandler),
            approval_required: false,
            approval_scope: ApprovalScope::default(),
            workspace: std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
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
            tools: ToolRegistry::new(),
            approval: Arc::new(AllowAllApprovalHandler),
            approval_required: false,
            approval_scope: ApprovalScope::default(),
            workspace: std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
        }
    }

    /// Slice 3 — inject a tool registry + approval handler.
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_approval(mut self, approval: Arc<dyn ApprovalHandler>) -> Self {
        self.approval = approval;
        self
    }

    pub fn with_approval_required(mut self, required: bool) -> Self {
        self.approval_required = required;
        self
    }

    pub fn with_approval_scope(mut self, scope: ApprovalScope) -> Self {
        self.approval_scope = scope;
        self
    }

    pub fn with_workspace(mut self, workspace: std::path::PathBuf) -> Self {
        self.workspace = workspace;
        self
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn register_tool(&mut self, tool: Arc<dyn zunel_tools::Tool>) {
        self.tools.register(tool);
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
        let history = session.get_history(HISTORY_LIMIT);
        let initial_messages = history_to_chat_messages(&history);
        let starting_len = initial_messages.len();

        tracing::debug!(
            model = %self.defaults.model,
            history_len = initial_messages.len(),
            "agent_loop: streaming",
        );

        let runner = AgentRunner::new(
            self.provider.clone(),
            self.tools.clone(),
            self.approval.clone(),
        );
        let result = runner
            .run(
                AgentRunSpec {
                    initial_messages,
                    model: self.defaults.model.clone(),
                    max_iterations: 15,
                    workspace: self.workspace.clone(),
                    session_key: session_key.into(),
                    approval_required: self.approval_required,
                    approval_scope: self.approval_scope,
                    hook: None,
                },
                sink,
            )
            .await?;

        for msg in result.messages.iter().skip(starting_len) {
            persist_runner_message(&mut session, msg);
        }
        sessions.save(&session)?;

        Ok(RunResult {
            content: result.content,
            tools_used: result.tools_used,
            messages: result.messages,
        })
    }
}

fn persist_runner_message(session: &mut Session, msg: &ChatMessage) {
    // Plain text turns serialize fine via add_message (it stamps a
    // timestamp). Tool messages and assistant turns carrying tool
    // calls need the wire-shaped JSON, which add_message can't
    // express, so we go through append_raw_message.
    if matches!(msg.role, Role::Tool) || !msg.tool_calls.is_empty() {
        session.append_raw_message(chat_message_to_value(msg));
    } else {
        let role = match msg.role {
            Role::User => ChatRole::User,
            Role::Assistant => ChatRole::Assistant,
            Role::System => ChatRole::System,
            Role::Tool => ChatRole::Tool,
        };
        session.add_message(role, &msg.content);
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
                tool_calls: Vec::new(),
            })
        })
        .collect()
}
