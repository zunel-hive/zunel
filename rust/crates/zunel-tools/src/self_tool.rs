use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

#[derive(Debug, Clone, Default)]
pub struct SelfState {
    pub model: String,
    pub provider: String,
    pub workspace: String,
    pub max_iterations: u32,
    pub current_iteration: u32,
    pub tools: Vec<String>,
    pub subagents: Vec<SubagentSummary>,
}

#[derive(Debug, Clone, Default)]
pub struct SubagentSummary {
    pub id: String,
    pub label: String,
    pub phase: String,
    pub iteration: u32,
}

pub struct SelfTool {
    state: SelfState,
}

impl SelfTool {
    pub fn new(state: SelfState) -> Self {
        Self { state }
    }

    fn render_summary(&self) -> String {
        let mut lines = vec![
            format!("model: {}", empty_as_unknown(&self.state.model)),
            format!("provider: {}", empty_as_unknown(&self.state.provider)),
            format!("workspace: {}", empty_as_unknown(&self.state.workspace)),
            format!(
                "iterations: {}/{}",
                self.state.current_iteration, self.state.max_iterations
            ),
            format!(
                "tools: {} registered - {}",
                self.state.tools.len(),
                self.state.tools.join(", ")
            ),
        ];
        if self.state.subagents.is_empty() {
            lines.push("subagents: none".into());
        } else {
            lines.push(format!("subagents: {}", self.state.subagents.len()));
            for subagent in &self.state.subagents {
                lines.push(format!(
                    "  [{}] {} - phase: {}, iteration: {}",
                    subagent.id, subagent.label, subagent.phase, subagent.iteration
                ));
            }
        }
        lines.join("\n")
    }
}

#[async_trait]
impl Tool for SelfTool {
    fn name(&self) -> &'static str {
        "self"
    }

    fn description(&self) -> &'static str {
        "Inspect safe read-only runtime state such as model, workspace, registered tools, and subagent status. Secrets are never exposed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["check", "set"]},
                "key": {"type": "string"},
                "value": {}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        match args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("check")
        {
            "check" => ToolResult::ok(self.render_summary()),
            "set" => ToolResult::err("self tool is read-only in Rust Slice 4"),
            other => ToolResult::err(format!("Unknown self action: {other}")),
        }
    }
}

fn empty_as_unknown(value: &str) -> &str {
    if value.is_empty() {
        "unknown"
    } else {
        value
    }
}
