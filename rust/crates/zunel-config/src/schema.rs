use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub providers: ProvidersConfig,
    pub agents: AgentsConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProvidersConfig {
    pub custom: Option<CustomProvider>,
    pub codex: Option<CodexProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProvider {
    pub api_key: String,
    pub api_base: String,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodexProvider {
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
    /// Python compat: ``agents.defaults.workspace`` in config.json. Default
    /// (``~/.zunel/workspace``) is applied at resolution time in
    /// ``workspace_path``, not in this struct — keeping ``AgentDefaults``
    /// round-trippable through serde without spurious values.
    pub workspace: Option<String>,
}

/// Slice 3 — opt-in configuration for tools and approvals.
///
/// Defaults are deliberately conservative: read-only filesystem and
/// search tools are seeded automatically by the agent layer, but `exec`,
/// `web_fetch`, and `web_search` are gated behind explicit `enable` flags
/// to match Python's parity behavior. The `approval_scope` field is a
/// plain string (`"all" | "shell" | "writes" | "none"`) so existing
/// configs round-trip cleanly; runtime code maps it to
/// `zunel_core::ApprovalScope`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub approval_required: bool,
    pub approval_scope: String,
    pub exec: ExecToolsConfig,
    pub web: WebToolsConfig,
    pub filesystem: FilesystemToolsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecToolsConfig {
    pub enable: bool,
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebToolsConfig {
    pub enable: bool,
    pub search_provider: String,
    pub brave_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesystemToolsConfig {
    pub media_dir: Option<PathBuf>,
}
