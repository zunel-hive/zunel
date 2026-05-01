use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub providers: ProvidersConfig,
    pub agents: AgentsConfig,
    pub channels: ChannelsConfig,
    pub gateway: GatewayConfig,
    pub tools: ToolsConfig,
    pub cli: CliConfig,
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
    #[serde(default, deserialize_with = "null_string_default")]
    pub api_key: String,
    #[serde(default, deserialize_with = "null_string_default")]
    pub api_base: String,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
}

fn null_string_default<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChannelsConfig {
    pub send_progress: bool,
    pub send_tool_hints: bool,
    pub send_max_retries: u8,
    /// When `true`, the gateway appends a one-line token-usage footer
    /// (e.g. `─ 312 in · 4 out · 1.2k session`) to every outbound
    /// channel message before publishing. Off by default — turn on for
    /// budget audits or per-turn cost visibility in Slack.
    pub show_token_footer: bool,
    pub slack: Option<SlackChannelConfig>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            send_progress: true,
            send_tool_hints: false,
            send_max_retries: 3,
            show_token_footer: false,
            slack: None,
        }
    }
}

/// CLI-side preferences that affect the local terminal experience but
/// are independent of channel/gateway behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CliConfig {
    /// When `true`, `zunel agent` and `zunel agent` REPL print the
    /// token-usage footer after each assistant reply. Can also be
    /// toggled per-invocation via `--show-tokens` regardless of this
    /// setting.
    pub show_token_footer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SlackChannelConfig {
    pub enabled: bool,
    pub mode: String,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
    pub allow_from: Vec<String>,
    pub group_policy: String,
    pub group_allow_from: Vec<String>,
    pub reply_in_thread: bool,
    pub react_emoji: Option<String>,
    pub done_emoji: Option<String>,
    pub dm: SlackDmConfig,
    /// When `true`, the built-in Slack MCP server hides the write tools
    /// (`slack_post_as_me`, `slack_dm_self`) from `tools/list` and refuses
    /// any direct calls into them. The agent never gets a handle to
    /// `chat.postMessage` on the user's token. Defaults to `false`
    /// (writes allowed) so the historical behavior of the standalone
    /// `zunel-mcp-slack` binary is preserved; flip to `true` to make the
    /// user OAuth token strictly read-only.
    pub user_token_read_only: bool,
    /// When non-empty (and `user_token_read_only = false`), the built-in
    /// Slack MCP server's write tools (`slack_post_as_me`, `slack_dm_self`)
    /// only permit posting to channel/user IDs in this list. Match is
    /// case-sensitive, against the literal Slack ID (e.g. `"U12F7K329"`,
    /// `"D0AUX99UNR0"`, `"C0123456789"`). An empty list means
    /// "no scope restriction" — any channel the token can reach. This
    /// composes with `user_token_read_only`: if read-only is on, the
    /// allowlist is irrelevant because the tools aren't exposed at all.
    pub write_allow: Vec<String>,
}

impl Default for SlackChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "socket".into(),
            bot_token: None,
            app_token: None,
            allow_from: Vec::new(),
            group_policy: "mention".into(),
            group_allow_from: Vec::new(),
            reply_in_thread: true,
            react_emoji: Some("eyes".into()),
            done_emoji: Some("white_check_mark".into()),
            dm: SlackDmConfig::default(),
            user_token_read_only: false,
            write_allow: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SlackDmConfig {
    pub enabled: bool,
    pub policy: String,
    pub allow_from: Vec<String>,
}

impl Default for SlackDmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "open".into(),
            allow_from: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub context_window_tokens: Option<u32>,
    pub context_block_limit: Option<u32>,
    pub max_tool_iterations: Option<usize>,
    pub max_tool_result_chars: Option<usize>,
    pub provider_retry_mode: Option<String>,
    pub timezone: Option<String>,
    pub unified_session: Option<bool>,
    pub disabled_skills: Vec<String>,
    pub session_ttl_minutes: Option<u32>,
    /// How many of the most recent unconsolidated session messages are
    /// replayed to the provider on each turn. Bounds the per-turn
    /// payload size for chat sessions that have accumulated long
    /// histories. ``None`` falls back to ``DEFAULT_SESSION_HISTORY_WINDOW``.
    pub session_history_window: Option<usize>,
    /// Idle-compaction trigger: when the gap between the previous user
    /// turn and the current one exceeds this many minutes, the loop
    /// LLM-summarizes everything older than ``compactionKeepTail``
    /// before sending the next request. ``None`` / ``0`` disables it.
    /// Accepts the legacy `idleCompactAfterMinutes` JSON key as an alias.
    #[serde(alias = "idleCompactAfterMinutes")]
    pub idle_compact_after_minutes: Option<u32>,
    /// How many recent messages a compaction pass leaves untouched.
    /// Defaults to ``DEFAULT_COMPACTION_KEEP_TAIL``.
    pub compaction_keep_tail: Option<usize>,
    /// Optional cheaper model used for `zunel sessions compact` and the
    /// idle-compaction trigger. Falls back to ``model`` when unset.
    pub compaction_model: Option<String>,
    pub dream: DreamConfig,
    /// Python compat: ``agents.defaults.workspace`` in config.json. Default
    /// (``~/.zunel/workspace``) is applied at resolution time in
    /// ``workspace_path``, not in this struct — keeping ``AgentDefaults``
    /// round-trippable through serde without spurious values.
    pub workspace: Option<String>,
}

/// Defaults for ``AgentDefaults.session_history_window``.
pub const DEFAULT_SESSION_HISTORY_WINDOW: usize = 40;
/// Defaults for ``AgentDefaults.compaction_keep_tail``.
pub const DEFAULT_COMPACTION_KEEP_TAIL: usize = 8;
/// Default tool-result char cap before snipping. Mirrors the previous
/// in-runner constant so untouched configs behave identically.
pub const DEFAULT_TOOL_RESULT_BUDGET_CHARS: usize = 16_000;
/// Default total context window in tokens. Mirrors the previous
/// in-runner constant so untouched configs behave identically.
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 65_536;
/// Tokens reserved for the model's reply on top of ``max_tokens`` when
/// computing the history snip budget.
pub const HISTORY_BUDGET_HEADROOM_TOKENS: u32 = 4_096;
/// Default ``max_tokens`` floor used when the agent default is unset
/// (matches the previous in-runner constant of 1024).
pub const DEFAULT_MAX_TOKENS_FALLBACK: u32 = 1_024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DreamConfig {
    pub interval_h: Option<u32>,
    pub model_override: Option<String>,
    pub max_batch_size: Option<u32>,
    pub max_iterations: Option<u32>,
    pub annotate_line_ages: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GatewayConfig {
    pub heartbeat: HeartbeatConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HeartbeatConfig {
    pub enabled: bool,
    pub interval_s: u64,
    pub keep_recent_messages: usize,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_s: 30 * 60,
            keep_recent_messages: 8,
        }
    }
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
    #[serde(rename = "mcpServers")]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecToolsConfig {
    pub enable: bool,
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
    /// Extra environment variables injected into every shell command
    /// the agent runs through `ExecTool`.
    ///
    /// Values support `${VAR}` and `${VAR:-default}` placeholders that
    /// expand against the gateway process's own environment at spawn
    /// time, so users can extend `PATH` rather than replace it:
    ///
    /// ```jsonc
    /// "tools": {
    ///   "exec": {
    ///     "env": { "PATH": "$HOME/.cargo/bin:${PATH}" }
    ///   }
    /// }
    /// ```
    ///
    /// Missing variables expand to the empty string (unlike the
    /// `mcpServers.<name>.headers` substitution, which drops the
    /// header entirely on a missing var) so a misconfigured `${VAR}`
    /// doesn't accidentally produce an unspawnable command. Use
    /// `${VAR:-fallback}` to be explicit when an empty expansion is
    /// dangerous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct McpServerConfig {
    #[serde(rename = "type")]
    pub transport_type: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub tool_timeout: Option<u64>,
    pub init_timeout: Option<u64>,
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_oauth")]
    pub oauth: Option<McpOAuthConfig>,
    #[serde(rename = "oauthScope")]
    pub oauth_scope: Option<String>,
    #[serde(rename = "oauthCallbackHost")]
    pub oauth_callback_host: Option<String>,
    #[serde(rename = "oauthCallbackPort")]
    pub oauth_callback_port: Option<u16>,
    #[serde(rename = "oauthClientId")]
    pub oauth_client_id: Option<String>,
    #[serde(rename = "oauthClientSecret")]
    pub oauth_client_secret: Option<String>,
    #[serde(rename = "oauthRedirectUri")]
    pub oauth_redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct McpOAuthConfig {
    pub enabled: bool,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub scope: Option<String>,
    pub callback_host: Option<String>,
    pub callback_port: Option<u16>,
    pub redirect_uri: Option<String>,
}

impl McpServerConfig {
    pub fn normalized_oauth(&self) -> Option<McpOAuthConfig> {
        let mut oauth = self.oauth.clone().unwrap_or_default();
        if self.oauth.is_none() && !self.has_legacy_oauth_fields() {
            return None;
        }
        oauth.scope = oauth.scope.or_else(|| self.oauth_scope.clone());
        oauth.callback_host = oauth
            .callback_host
            .or_else(|| self.oauth_callback_host.clone());
        oauth.callback_port = oauth.callback_port.or(self.oauth_callback_port);
        oauth.client_id = oauth.client_id.or_else(|| self.oauth_client_id.clone());
        oauth.client_secret = oauth
            .client_secret
            .or_else(|| self.oauth_client_secret.clone());
        oauth.redirect_uri = oauth
            .redirect_uri
            .or_else(|| self.oauth_redirect_uri.clone());
        Some(oauth)
    }

    fn has_legacy_oauth_fields(&self) -> bool {
        self.oauth_scope.is_some()
            || self.oauth_callback_host.is_some()
            || self.oauth_callback_port.is_some()
            || self.oauth_client_id.is_some()
            || self.oauth_client_secret.is_some()
            || self.oauth_redirect_uri.is_some()
    }
}

fn deserialize_oauth<'de, D>(deserializer: D) -> Result<Option<McpOAuthConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Bool(enabled) => Ok(Some(McpOAuthConfig {
            enabled,
            ..Default::default()
        })),
        other => serde_json::from_value(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}
