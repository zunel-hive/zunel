//! Approval handler trait + per-session cache.
//!
//! Python parity: `zunel/agent/approval.py`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalScope {
    #[default]
    All,
    Shell,
    Writes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool_name: String,
    pub args: Value,
    pub description: String,
    pub scope: ApprovalScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[async_trait]
pub trait ApprovalHandler: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision;
}

/// Stable cache key for `(tool, args)`. JSON-keys are sorted so the
/// same tool call produces the same hash regardless of argument
/// iteration order.
pub fn summarize_tool_call(tool: &str, args: &Value) -> String {
    let sorted = sort_json(args);
    let serialized = serde_json::to_string(&sorted).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update(b":");
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sort_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(String, Value)> =
                map.iter().map(|(k, v)| (k.clone(), sort_json(v))).collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_json).collect()),
        other => other.clone(),
    }
}

/// Given a tool name + desired scope, returns whether running the
/// tool requires user consent.
pub fn tool_requires_approval(tool_name: &str, scope: ApprovalScope) -> bool {
    const SHELL_TOOLS: &[&str] = &["exec"];
    const WRITE_TOOLS: &[&str] = &["exec", "write_file", "edit_file"];
    match scope {
        ApprovalScope::All => true,
        ApprovalScope::Shell => SHELL_TOOLS.contains(&tool_name),
        ApprovalScope::Writes => WRITE_TOOLS.contains(&tool_name),
    }
}

/// Wraps another `ApprovalHandler` with a per-call cache. The cache
/// key is `summarize_tool_call(req.tool_name, &req.args)`.
pub struct CachedApprovalHandler {
    inner: Arc<dyn ApprovalHandler>,
    cache: Mutex<HashMap<String, ApprovalDecision>>,
}

impl CachedApprovalHandler {
    pub fn new(inner: Arc<dyn ApprovalHandler>) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ApprovalHandler for CachedApprovalHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        let key = summarize_tool_call(&req.tool_name, &req.args);
        if let Some(d) = self.cache.lock().unwrap().get(&key).copied() {
            return d;
        }
        let decision = self.inner.request(req).await;
        self.cache.lock().unwrap().insert(key, decision);
        decision
    }
}

/// Always-approve handler (used as the default when approval is off).
pub struct AllowAllApprovalHandler;

#[async_trait]
impl ApprovalHandler for AllowAllApprovalHandler {
    async fn request(&self, _req: ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approve
    }
}
