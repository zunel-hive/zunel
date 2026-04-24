//! Agent runner: drives a single turn's provider-stream ↔ tool-call loop.
//!
//! Python parity: `zunel/agent/runner.py::AgentRunner`.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;

use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, Role, StreamEvent, ToolCallAccumulator,
    ToolCallRequest,
};
use zunel_tools::{ToolContext, ToolRegistry};

use crate::approval::{
    tool_requires_approval, ApprovalDecision, ApprovalHandler, ApprovalRequest, ApprovalScope,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Completed,
    MaxIterations,
    Error,
    ToolError,
    EmptyFinalResponse,
}

pub struct AgentRunSpec {
    /// System + bootstrap + skills prompt plus the turn's user message.
    /// Runner mutates a working copy; callers that need the full
    /// post-run history read `AgentRunResult::messages`.
    pub initial_messages: Vec<ChatMessage>,
    pub model: String,
    pub max_iterations: usize,
    pub workspace: std::path::PathBuf,
    pub session_key: String,
    pub approval_required: bool,
    pub approval_scope: ApprovalScope,
}

impl Default for AgentRunSpec {
    fn default() -> Self {
        Self {
            initial_messages: Vec::new(),
            model: String::new(),
            max_iterations: 15,
            workspace: std::env::temp_dir(),
            session_key: String::new(),
            approval_required: false,
            approval_scope: ApprovalScope::default(),
        }
    }
}

pub struct AgentRunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
    pub stop_reason: StopReason,
}

pub struct AgentRunner {
    provider: Arc<dyn LLMProvider>,
    tools: ToolRegistry,
    approval: Arc<dyn ApprovalHandler>,
}

impl AgentRunner {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        tools: ToolRegistry,
        approval: Arc<dyn ApprovalHandler>,
    ) -> Self {
        Self {
            provider,
            tools,
            approval,
        }
    }

    pub async fn run(
        &self,
        spec: AgentRunSpec,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<AgentRunResult, crate::Error> {
        let mut messages = spec.initial_messages.clone();
        let mut tools_used: Vec<String> = Vec::new();
        let ctx = ToolContext::new_with_workspace(spec.workspace.clone(), spec.session_key.clone());

        let max_iter = if spec.max_iterations == 0 {
            15
        } else {
            spec.max_iterations
        };
        let settings = GenerationSettings::default();
        let tool_defs: Vec<zunel_providers::ToolSchema> = self
            .tools
            .get_definitions()
            .into_iter()
            .map(schema_from_definition)
            .collect();

        let mut last_content = String::new();
        let mut stop = StopReason::Error;

        'outer: for iteration in 0..max_iter {
            tracing::debug!(iteration, "agent iteration");
            let (content, finish_reason, calls) = {
                let stream =
                    self.provider
                        .generate_stream(&spec.model, &messages, &tool_defs, &settings);
                futures::pin_mut!(stream);
                let mut acc = ToolCallAccumulator::default();
                let mut content = String::new();
                let mut finish_reason: Option<String> = None;
                while let Some(event) = stream.next().await {
                    let event = event.map_err(crate::Error::Provider)?;
                    let _ = sink.send(event.clone()).await;
                    match &event {
                        StreamEvent::ContentDelta(s) => content.push_str(s),
                        StreamEvent::Done(resp) => finish_reason = resp.finish_reason.clone(),
                        _ => {}
                    }
                    acc.push(event);
                }
                let calls = acc
                    .finalize()
                    .map_err(|e| crate::Error::ToolCallAssembly(e.to_string()))?;
                (content, finish_reason, calls)
            };

            if calls.is_empty() {
                stop = if content.is_empty() && finish_reason.as_deref() != Some("length") {
                    StopReason::EmptyFinalResponse
                } else {
                    StopReason::Completed
                };
                last_content = content.clone();
                if !content.is_empty() {
                    messages.push(ChatMessage {
                        role: Role::Assistant,
                        content,
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    });
                }
                break 'outer;
            }

            messages.push(ChatMessage {
                role: Role::Assistant,
                content: content.clone(),
                tool_call_id: None,
                tool_calls: calls.clone(),
            });

            for call in &calls {
                tools_used.push(call.name.clone());
                if spec.approval_required && tool_requires_approval(&call.name, spec.approval_scope)
                {
                    let req = ApprovalRequest {
                        tool_name: call.name.clone(),
                        args: call.arguments.clone(),
                        description: describe_call(call),
                        scope: spec.approval_scope,
                    };
                    match self.approval.request(req).await {
                        ApprovalDecision::Approve => {}
                        ApprovalDecision::Deny => {
                            messages.push(tool_result_message(
                                &call.id,
                                &call.name,
                                "denied by user",
                            ));
                            continue;
                        }
                    }
                }
                let result = self
                    .tools
                    .execute(&call.name, call.arguments.clone(), &ctx)
                    .await
                    .expect("registry never fails");
                messages.push(tool_result_message(&call.id, &call.name, &result.content));
            }

            if iteration + 1 == max_iter {
                stop = StopReason::MaxIterations;
            }
        }

        Ok(AgentRunResult {
            content: last_content,
            tools_used,
            messages,
            stop_reason: stop,
        })
    }
}

fn tool_result_message(tool_call_id: &str, _name: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: Role::Tool,
        content: content.to_string(),
        tool_call_id: Some(tool_call_id.to_string()),
        tool_calls: Vec::new(),
    }
}

fn describe_call(tc: &ToolCallRequest) -> String {
    format!("{}({})", tc.name, tc.arguments)
}

fn schema_from_definition(def: serde_json::Value) -> zunel_providers::ToolSchema {
    let function = def
        .get("function")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    zunel_providers::ToolSchema {
        name: function
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        description: function
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        parameters: function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    }
}
