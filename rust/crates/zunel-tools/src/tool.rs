use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::file_state::FileStateTracker;

/// Per-call context a tool can read.
///
/// Slice 3 exposes the workspace, the session key, a cancellation
/// token so long-running tools (`exec`, `web_fetch`) can be aborted
/// when the parent agent loop is cancelled, and a `FileStateTracker`
/// shared between read/write/edit tools.
#[derive(Clone)]
pub struct ToolContext {
    pub workspace: PathBuf,
    pub session_key: String,
    pub cancel: tokio_util::sync::CancellationToken,
    pub file_state: FileStateTracker,
}

impl ToolContext {
    pub fn new_with_workspace(workspace: PathBuf, session_key: String) -> Self {
        Self {
            workspace,
            session_key,
            cancel: tokio_util::sync::CancellationToken::new(),
            file_state: FileStateTracker::default(),
        }
    }

    /// Build a throw-away context for tests.
    pub fn for_test() -> Self {
        Self::new_with_workspace(std::env::temp_dir(), "cli:direct".into())
    }
}

/// Uniform return type for tool execution. `is_error` mirrors
/// Python's `ToolResult.is_error` and is true when the tool raised —
/// the runner appends the content as a tool message either way, the
/// flag only drives the `tools_used` stat and logging color.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON Schema `function.parameters` object.
    fn parameters(&self) -> Value;
    /// Whether this tool is safe to run concurrently with other
    /// `concurrency_safe` tools in the same batch.
    fn concurrency_safe(&self) -> bool {
        false
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}

pub type DynTool = Arc<dyn Tool>;
