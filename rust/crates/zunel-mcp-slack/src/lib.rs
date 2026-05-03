//! Built-in Slack MCP tool surface.
//!
//! This module exposes the Slack tool catalog and dispatcher as a library so
//! both the standalone `zunel-mcp-slack` binary and `zunel mcp serve --server
//! slack` (in the `zunel-cli` crate) can register the same tools without
//! duplicating logic.
//!
//! Two safety knobs in `~/.zunel/config.json` shape what's exposed:
//!
//! - `channels.slack.userTokenReadOnly` — when `true`, the write tools
//!   (`slack_post_as_me`, `slack_dm_self`) are filtered out of [`tools`] and
//!   refused at [`call_tool`] as a defense-in-depth measure. Even a host
//!   that builds its own tool catalog cannot post on the user's behalf.
//! - `channels.slack.writeAllow` — when non-empty (and read-only is off),
//!   the write tools only permit posting to literal Slack channel/user IDs
//!   in the list. The agent can post to itself or to a designated incident
//!   channel, but not to arbitrary teammates.

use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Names of the Slack tools that perform write actions on behalf of the
/// authenticated user (i.e. `chat.postMessage`). These are gated by the
/// `channels.slack.userTokenReadOnly` and `channels.slack.writeAllow`
/// config knobs.
const WRITE_TOOLS: &[&str] = &["slack_post_as_me", "slack_dm_self"];

/// Resolved safety posture for the Slack write surface, derived from
/// `channels.slack.*` once per dispatch. Centralizes the config read so
/// `tools()` and `call_tool()` agree on what's allowed without re-parsing
/// the config file twice in the same RPC. Public so introspection callers
/// (notably `zunel-mcp-self`'s `zunel_slack_capability` tool) can report
/// the live posture without re-implementing the same logic.
#[derive(Debug, Clone, Default)]
pub struct SlackSafety {
    /// `channels.slack.userTokenReadOnly`. When `true`, write tools are
    /// hidden and refused regardless of `write_allow`.
    pub read_only: bool,
    /// `channels.slack.writeAllow`. Empty means "no scope restriction"
    /// (any channel the token can reach). Non-empty restricts writes to
    /// the listed channel/user IDs.
    pub write_allow: Vec<String>,
}

impl SlackSafety {
    pub fn write_allowed_to(&self, target: &str) -> bool {
        if self.read_only {
            return false;
        }
        if self.write_allow.is_empty() {
            return true;
        }
        self.write_allow.iter().any(|allowed| allowed == target)
    }
}

/// Return the JSON schemas for every Slack MCP tool the runtime should expose
/// to the agent. Honors `channels.slack.userTokenReadOnly`: when read-only is
/// in effect, [`WRITE_TOOLS`] are filtered out so the agent never sees them.
/// `writeAllow` is enforced at call time rather than tools-list time because
/// the allowed targets are call arguments, not tool names — hiding the tool
/// would block the legitimate "post to my one allowed channel" path.
pub fn tools() -> Vec<Value> {
    let safety = load_safety();
    full_tool_catalog()
        .into_iter()
        .filter(|tool| {
            !safety.read_only
                || tool
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| !WRITE_TOOLS.contains(&name))
        })
        .collect()
}

/// Dispatch a single `tools/call` for the Slack MCP server.
///
/// Returns the tool's text payload on success; on failure returns a JSON
/// payload describing the error. Write tools are refused when read-only
/// mode is active or when the target falls outside `writeAllow`,
/// regardless of how the caller framed the request.
pub async fn call_tool(name: &str, args: &Value) -> Result<String> {
    let safety = load_safety();
    if WRITE_TOOLS.contains(&name) && safety.read_only {
        return Ok(serde_json::to_string(&json!({
            "ok": false,
            "error": "user_token_read_only",
            "hint": "Set channels.slack.userTokenReadOnly = false in ~/.zunel/config.json to allow this tool to post on the user's behalf."
        }))?);
    }
    match name {
        "slack_whoami" => Ok(slack_whoami()),
        "slack_channel_history" => {
            let channel = required_str(args, "channel")?;
            let mut params = vec![
                ("channel".to_string(), channel.to_string()),
                ("limit".to_string(), limit_arg(args, 50).to_string()),
            ];
            push_optional(args, &mut params, "oldest");
            push_optional(args, &mut params, "latest");
            push_optional(args, &mut params, "cursor");
            let data = slack_api_call("conversations.history", params).await?;
            Ok(render_history(data, channel))
        }
        "slack_channel_replies" => {
            let channel = required_str(args, "channel")?;
            let ts = required_str(args, "ts")?;
            let mut params = vec![
                ("channel".to_string(), channel.to_string()),
                ("ts".to_string(), ts.to_string()),
                ("limit".to_string(), limit_arg(args, 50).to_string()),
            ];
            push_optional(args, &mut params, "cursor");
            let data = slack_api_call("conversations.replies", params).await?;
            Ok(render_history(data, channel))
        }
        "slack_search_messages" => {
            let data = search_call(args, vec!["messages"]).await?;
            Ok(render_search_messages(data))
        }
        "slack_search_users" => {
            let data = search_call(args, vec!["users"]).await?;
            Ok(render_search_users(data))
        }
        "slack_search_files" => {
            let data = search_call(args, vec!["files"]).await?;
            Ok(render_search_files(data))
        }
        "slack_list_users" => {
            let mut params = vec![("limit".to_string(), limit_arg(args, 50).to_string())];
            push_optional(args, &mut params, "cursor");
            let data = slack_api_call("users.list", params).await?;
            Ok(render_users_list(data))
        }
        "slack_user_info" => {
            let data = slack_api_call(
                "users.info",
                vec![("user".to_string(), required_str(args, "user")?.to_string())],
            )
            .await?;
            Ok(render_user_info(data))
        }
        "slack_permalink" => {
            let data = slack_api_call(
                "chat.getPermalink",
                vec![
                    (
                        "channel".to_string(),
                        required_str(args, "channel")?.to_string(),
                    ),
                    (
                        "message_ts".to_string(),
                        required_str(args, "message_ts")?.to_string(),
                    ),
                ],
            )
            .await?;
            Ok(if data.get("ok").and_then(Value::as_bool) == Some(true) {
                serde_json::to_string(&json!({
                    "ok": true,
                    "permalink": data.get("permalink").cloned().unwrap_or(Value::Null)
                }))?
            } else {
                serde_json::to_string(&data)?
            })
        }
        "slack_post_as_me" => slack_post_as_me(args, &safety).await,
        "slack_dm_self" => {
            let text = required_str(args, "text")?;
            if text.trim().is_empty() {
                return Ok(serde_json::to_string(
                    &json!({"ok": false, "error": "empty_text"}),
                )?);
            }
            let user_id = slack_token_user_id().context("could not resolve Slack user_id")?;
            slack_post_as_me(&json!({"channel": user_id, "text": text}), &safety).await
        }
        _ => Ok(format!("unknown tool: {name}")),
    }
}

fn full_tool_catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "slack_whoami",
            "description": "Report Slack MCP authentication status without exposing tokens",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "slack_channel_history",
            "description": "Read recent messages from a Slack channel",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": {"type": "string"},
                    "limit": {"type": "integer"},
                    "oldest": {"type": "string"},
                    "latest": {"type": "string"},
                    "cursor": {"type": "string"}
                },
                "required": ["channel"]
            }
        }),
        json!({
            "name": "slack_search_messages",
            "description": "Search Slack messages using assistant.search.context",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                    "channel_types": {"type": "array", "items": {"type": "string"}},
                    "after": {"type": "integer"},
                    "before": {"type": "integer"},
                    "include_context_messages": {"type": "boolean"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "slack_search_users",
            "description": "Search Slack users using assistant.search.context",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "slack_search_files",
            "description": "Search Slack files using assistant.search.context",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                    "channel_types": {"type": "array", "items": {"type": "string"}},
                    "after": {"type": "integer"},
                    "before": {"type": "integer"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "slack_channel_replies",
            "description": "Read replies in a Slack thread",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": {"type": "string"},
                    "ts": {"type": "string"},
                    "limit": {"type": "integer"},
                    "cursor": {"type": "string"}
                },
                "required": ["channel", "ts"]
            }
        }),
        json!({
            "name": "slack_list_users",
            "description": "List Slack workspace members",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer"},
                    "cursor": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "slack_user_info",
            "description": "Look up a Slack user by ID",
            "inputSchema": {
                "type": "object",
                "properties": {"user": {"type": "string"}},
                "required": ["user"]
            }
        }),
        json!({
            "name": "slack_permalink",
            "description": "Get a Slack message permalink",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": {"type": "string"},
                    "message_ts": {"type": "string"}
                },
                "required": ["channel", "message_ts"]
            }
        }),
        json!({
            "name": "slack_post_as_me",
            "description": "Post a Slack message as the authenticated user. Requires channels.slack.userTokenReadOnly = false in ~/.zunel/config.json. If channels.slack.writeAllow is set, the target channel/user ID must be on the list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": {"type": "string"},
                    "text": {"type": "string"},
                    "thread_ts": {"type": "string"}
                },
                "required": ["channel", "text"]
            }
        }),
        json!({
            "name": "slack_dm_self",
            "description": "Post a Slack message to the authenticated user's self-DM. Requires channels.slack.userTokenReadOnly = false. If channels.slack.writeAllow is set, the authenticated user_id must be on the list.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }
        }),
    ]
}

/// Render a JSON introspection report covering the live Slack MCP surface.
///
/// Centralized here (instead of in the consuming `self` MCP servers) so
/// both `zunel-mcp-self` and `zunel-cli`'s `mcp serve --server self`
/// dispatcher emit identical payloads without copy/pasting the logic. The
/// payload is what the agent's `zunel_slack_capability` tool returns.
///
/// Reports:
/// - `tool_names` — the live tool list from [`tools`], already filtered
///   by `userTokenReadOnly` (so a read-only host shows no
///   `slack_post_as_me` / `slack_dm_self`).
/// - `tool_count` and `write_tools_exposed` — convenience flags that
///   mirror the filter outcome.
/// - `user_token_present` — whether the cached user OAuth token at
///   `~/.zunel/slack-app-mcp/user_token.json` exists and looks like a
///   real `xoxp-`/`xoxe.xoxp-` token. Doesn't expose the token itself.
/// - `safety` — `{user_token_read_only, write_allow_count,
///   write_allow_sample}`. The sample is capped to 5 entries so a host
///   with a long allowlist can't accidentally page-blow the agent's
///   context window via this introspection tool.
pub fn capability_report() -> String {
    let live_tools = tools();
    let tool_names: Vec<&str> = live_tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();
    let write_tools_exposed = tool_names
        .iter()
        .any(|name| matches!(*name, "slack_post_as_me" | "slack_dm_self"));

    let safety = load_safety();
    let write_allow_count = safety.write_allow.len();
    let write_allow_sample: Vec<&str> = safety
        .write_allow
        .iter()
        .take(5)
        .map(String::as_str)
        .collect();

    let token_path = zunel_config::zunel_home()
        .ok()
        .map(|home| home.join("slack-app-mcp").join("user_token.json"));
    let user_token_present = token_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("access_token")
                .and_then(Value::as_str)
                .map(|token| token.starts_with("xoxp-") || token.starts_with("xoxe.xoxp-"))
        })
        .unwrap_or(false);

    serde_json::to_string(&json!({
        "tool_names": tool_names,
        "tool_count": tool_names.len(),
        "write_tools_exposed": write_tools_exposed,
        "user_token_present": user_token_present,
        "user_token_path": token_path.as_ref().map(|p| p.display().to_string()),
        "safety": {
            "user_token_read_only": safety.read_only,
            "write_allow_count": write_allow_count,
            "write_allow_sample": write_allow_sample,
        }
    }))
    .unwrap_or_else(|_| "{}".into())
}

/// JSON-Schema descriptor for the `zunel_slack_capability` tool. Hosts
/// that ship the `self` server alongside the slack server (i.e. the
/// `zunel-mcp-self` binary and `zunel mcp serve --server self`) register
/// this in their tools-list so the agent can discover the introspection
/// surface.
pub fn capability_tool_descriptor() -> Value {
    json!({
        "name": "zunel_slack_capability",
        "description": "Report what the built-in Slack MCP can actually do right now: live tool names, whether a user OAuth token is cached, and the user-token safety posture (userTokenReadOnly, writeAllow). Useful when the agent is asked 'can you post to Slack?' so it can answer from runtime truth instead of guessing.",
        "inputSchema": {"type": "object", "properties": {}}
    })
}

/// Resolve the live `channels.slack.*` safety posture from disk. Falls back
/// to a permissive default (writes allowed, no allowlist) on any read/parse
/// error so that simple invocations without a config still expose the full
/// surface and keep the unit tests deterministic. Hosts that want to
/// enforce read-only or allowlist-scoped writes must set the flags
/// explicitly in `~/.zunel/config.json`.
pub fn load_safety() -> SlackSafety {
    zunel_config::load_config(None)
        .ok()
        .and_then(|cfg| cfg.channels.slack)
        .map(|slack| SlackSafety {
            read_only: slack.user_token_read_only,
            write_allow: slack.write_allow,
        })
        .unwrap_or_default()
}

async fn slack_post_as_me(args: &Value, safety: &SlackSafety) -> Result<String> {
    let channel = required_str(args, "channel")?;
    let text = required_str(args, "text")?;
    if text.trim().is_empty() {
        return Ok(serde_json::to_string(
            &json!({"ok": false, "error": "empty_text"}),
        )?);
    }
    if !safety.write_allowed_to(channel) {
        return Ok(serde_json::to_string(&json!({
            "ok": false,
            "error": "channel_not_in_write_allow",
            "channel": channel,
            "hint": "Add this Slack channel/user ID to channels.slack.writeAllow in ~/.zunel/config.json (or empty the list to remove the scope restriction).",
            "write_allow": safety.write_allow.clone()
        }))?);
    }
    let mut params = vec![
        ("channel".to_string(), channel.to_string()),
        ("text".to_string(), text.to_string()),
    ];
    push_optional(args, &mut params, "thread_ts");
    let data = slack_api_call("chat.postMessage", params).await?;
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(serde_json::to_string(&data)?);
    }
    let posted_channel = data
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(channel);
    let ts = data.get("ts").and_then(Value::as_str).unwrap_or_default();
    let permalink = if ts.is_empty() {
        Value::Null
    } else {
        let link = slack_api_call(
            "chat.getPermalink",
            vec![
                ("channel".to_string(), posted_channel.to_string()),
                ("message_ts".to_string(), ts.to_string()),
            ],
        )
        .await?;
        link.get("permalink").cloned().unwrap_or(Value::Null)
    };
    Ok(serde_json::to_string(&json!({
        "ok": true,
        "channel": posted_channel,
        "ts": ts,
        "permalink": permalink
    }))?)
}

async fn search_call(args: &Value, content_types: Vec<&str>) -> Result<Value> {
    let mut payload = serde_json::Map::new();
    payload.insert("query".into(), json!(required_str(args, "query")?));
    payload.insert("content_types".into(), json!(content_types));
    payload.insert("limit".into(), json!(limit_arg(args, 20)));
    if args
        .get("channel_types")
        .and_then(Value::as_array)
        .is_some_and(|types| !types.is_empty())
    {
        payload.insert("channel_types".into(), args["channel_types"].clone());
    } else if payload
        .get("content_types")
        .and_then(Value::as_array)
        .is_some_and(|types| {
            types
                .iter()
                .any(|ty| matches!(ty.as_str(), Some("messages" | "files")))
        })
    {
        payload.insert(
            "channel_types".into(),
            json!(["public_channel", "private_channel", "mpim", "im"]),
        );
    }
    if let Some(after) = args.get("after").and_then(Value::as_i64) {
        payload.insert("after".into(), json!(after));
    }
    if let Some(before) = args.get("before").and_then(Value::as_i64) {
        payload.insert("before".into(), json!(before));
    }
    if args
        .get("include_context_messages")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        payload.insert("include_context_messages".into(), json!(true));
    }
    slack_api_json_call("assistant.search.context", Value::Object(payload)).await
}

fn slack_whoami() -> String {
    if std::env::var("SLACK_USER_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .is_some()
        || slack_token_file_payload().is_ok_and(|value| {
            value
                .get("access_token")
                .and_then(Value::as_str)
                .is_some_and(|token| token.starts_with("xoxp-") || token.starts_with("xoxe.xoxp-"))
        })
    {
        "slack token configured".into()
    } else {
        "slack token missing".into()
    }
}

async fn slack_api_call(method: &str, params: Vec<(String, String)>) -> Result<Value> {
    let token = slack_token().await?;
    let base = std::env::var("SLACK_API_BASE").unwrap_or_else(|_| "https://slack.com".into());
    let url = format!("{}/api/{method}", base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(token)
        .form(&params)
        .send()
        .await?
        .error_for_status()?;
    let mut payload: Value = response.json().await?;
    if payload.get("error").and_then(Value::as_str) == Some("token_expired") {
        match force_refresh_user_token().await? {
            ForceRefreshOutcome::Token(token) => {
                let retry = client
                    .post(format!("{}/api/{method}", base.trim_end_matches('/')))
                    .bearer_auth(token)
                    .form(&params)
                    .send()
                    .await?
                    .error_for_status()?;
                return Ok(retry.json().await?);
            }
            ForceRefreshOutcome::SlackRejected(err) => {
                annotate_token_expired(&mut payload, &err);
            }
            ForceRefreshOutcome::Unavailable => {}
        }
    }
    Ok(payload)
}

async fn slack_api_json_call(method: &str, body: Value) -> Result<Value> {
    let token = slack_token().await?;
    let base = std::env::var("SLACK_API_BASE").unwrap_or_else(|_| "https://slack.com".into());
    let url = format!("{}/api/{method}", base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut payload: Value = response.json().await?;
    if payload.get("error").and_then(Value::as_str) == Some("token_expired") {
        match force_refresh_user_token().await? {
            ForceRefreshOutcome::Token(token) => {
                let retry = client
                    .post(format!("{}/api/{method}", base.trim_end_matches('/')))
                    .bearer_auth(token)
                    .json(&body)
                    .send()
                    .await?
                    .error_for_status()?;
                return Ok(retry.json().await?);
            }
            ForceRefreshOutcome::SlackRejected(err) => {
                annotate_token_expired(&mut payload, &err);
            }
            ForceRefreshOutcome::Unavailable => {}
        }
    }
    Ok(payload)
}

fn annotate_token_expired(payload: &mut Value, slack_err: &str) {
    payload["error"] = json!(format!(
        "token_expired (refresh failed: {slack_err}; run `zunel slack login --force` to re-mint the user token)"
    ));
}

async fn slack_token() -> Result<String> {
    if let Some(token) = std::env::var("SLACK_USER_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
    {
        return Ok(token);
    }
    let path = slack_token_file_path()?;
    let mut value = slack_token_file_payload_at(&path)?;
    let _ = maybe_refresh_user_token(&path, &mut value, false).await?;
    value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .filter(|token| token.starts_with("xoxp-") || token.starts_with("xoxe.xoxp-"))
        .map(str::to_string)
        .context("Slack user token file is missing a user access_token")
}

fn slack_token_user_id() -> Result<String> {
    let value = slack_token_file_payload()?;
    value
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|user_id| !user_id.is_empty())
        .map(str::to_string)
        .context("Slack user token file is missing user_id")
}

fn slack_token_file_payload() -> Result<Value> {
    let path = slack_token_file_path()?;
    slack_token_file_payload_at(&path)
}

fn slack_token_file_path() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("ZUNEL_SLACK_USER_TOKEN_PATH") {
        return Ok(std::path::PathBuf::from(path));
    }
    Ok(zunel_config::zunel_home()?
        .join("slack-app-mcp")
        .join("user_token.json"))
}

fn slack_token_file_payload_at(path: &std::path::Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("SLACK_USER_TOKEN is required or read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// What `force_refresh_user_token` produced for the API-call recovery path.
///
/// The two interesting cases for the caller are `Token` (retry) and
/// `SlackRejected` (annotate the original `token_expired` payload with the
/// underlying refresh error so users know to re-run `zunel slack login`).
enum ForceRefreshOutcome {
    /// Refresh succeeded; here is the new bearer token to retry with.
    Token(String),
    /// Slack rejected the refresh with this `error` code (e.g.
    /// `invalid_refresh_token`). The cached file was left untouched.
    SlackRejected(String),
    /// Refresh wasn't even attempted (env override, no `refresh_token`
    /// cached, etc.). Nothing actionable for the caller to surface.
    Unavailable,
}

async fn force_refresh_user_token() -> Result<ForceRefreshOutcome> {
    if std::env::var("SLACK_USER_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .is_some()
    {
        return Ok(ForceRefreshOutcome::Unavailable);
    }
    let path = slack_token_file_path()?;
    let mut value = slack_token_file_payload_at(&path)?;
    let outcome = maybe_refresh_user_token(&path, &mut value, true).await?;
    match outcome {
        RefreshOutcome::Refreshed => {
            let token = value
                .get("access_token")
                .and_then(Value::as_str)
                .filter(|token| token.starts_with("xoxp-") || token.starts_with("xoxe.xoxp-"))
                .map(str::to_string);
            Ok(token
                .map(ForceRefreshOutcome::Token)
                .unwrap_or(ForceRefreshOutcome::Unavailable))
        }
        RefreshOutcome::SlackRejected(err) => Ok(ForceRefreshOutcome::SlackRejected(err)),
        RefreshOutcome::NotAttempted => Ok(ForceRefreshOutcome::Unavailable),
    }
}

/// What `maybe_refresh_user_token` did with the cached user token file.
enum RefreshOutcome {
    /// No oauth.v2.access call was made (env override, no `refresh_token`
    /// cached, or non-`force` mode with a still-fresh token).
    NotAttempted,
    /// Slack returned a fresh access token; the file has been updated.
    Refreshed,
    /// `oauth.v2.access` returned `{"ok":false,"error":"…"}`; this is the
    /// underlying error string so callers can surface it.
    SlackRejected(String),
}

async fn maybe_refresh_user_token(
    path: &std::path::Path,
    value: &mut Value,
    force: bool,
) -> Result<RefreshOutcome> {
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    let expires_at = value.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    if refresh_token.is_none()
        || (!force && (expires_at == 0 || current_epoch_secs() + 60 < expires_at))
    {
        return Ok(RefreshOutcome::NotAttempted);
    }
    let Some(refresh_token) = refresh_token else {
        return Ok(RefreshOutcome::NotAttempted);
    };
    let app_info_path = path
        .parent()
        .context("Slack token path has no parent")?
        .join("app_info.json");
    let app_info: Value =
        serde_json::from_str(&std::fs::read_to_string(&app_info_path).with_context(|| {
            format!(
                "reading {} for Slack token refresh",
                app_info_path.display()
            )
        })?)
        .with_context(|| format!("parsing {}", app_info_path.display()))?;
    let client_id = app_info
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("app_info.json missing client_id")?;
    let client_secret = app_info
        .get("client_secret")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("app_info.json missing client_secret")?;
    let base = std::env::var("SLACK_API_BASE").unwrap_or_else(|_| "https://slack.com".into());
    let data: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/oauth.v2.access",
            base.trim_end_matches('/')
        ))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .context("refreshing Slack user token")?
        .error_for_status()
        .context("refreshing Slack user token")?
        .json()
        .await
        .context("decoding Slack token refresh response")?;
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        let err = data
            .get("error")
            .and_then(Value::as_str)
            .filter(|err| !err.is_empty())
            .unwrap_or("unknown_error")
            .to_string();
        return Ok(RefreshOutcome::SlackRejected(err));
    }
    let authed_user = data
        .get("authed_user")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let new_access = authed_user
        .get("access_token")
        .and_then(Value::as_str)
        .or_else(|| {
            (data.get("token_type").and_then(Value::as_str) == Some("user"))
                .then(|| data.get("access_token").and_then(Value::as_str))
                .flatten()
        })
        .unwrap_or_default();
    let expires_in = authed_user
        .get("expires_in")
        .and_then(Value::as_i64)
        .or_else(|| data.get("expires_in").and_then(Value::as_i64))
        .unwrap_or(0);
    if !(new_access.starts_with("xoxp-") || new_access.starts_with("xoxe.xoxp-")) || expires_in <= 0
    {
        // Slack said `ok: true` but the response shape doesn't match the
        // user-token grant we expect. Treat that as an unactionable
        // rejection so the caller can surface a useful hint.
        return Ok(RefreshOutcome::SlackRejected(
            "ok_but_no_user_access_token".to_string(),
        ));
    }
    value["access_token"] = json!(new_access);
    value["refresh_token"] = json!(authed_user
        .get("refresh_token")
        .and_then(Value::as_str)
        .or_else(|| data.get("refresh_token").and_then(Value::as_str))
        .unwrap_or(refresh_token.as_str()));
    value["expires_at"] = json!(current_epoch_secs() + expires_in);
    if let Some(user_id) = authed_user.get("id").and_then(Value::as_str) {
        value["user_id"] = json!(user_id);
    }
    if let Some(team_id) = data.pointer("/team/id").and_then(Value::as_str) {
        value["team_id"] = json!(team_id);
    }
    if let Some(team_name) = data.pointer("/team/name").and_then(Value::as_str) {
        value["team_name"] = json!(team_name);
    }
    if let Some(enterprise_id) = data.pointer("/enterprise/id").and_then(Value::as_str) {
        value["enterprise_id"] = json!(enterprise_id);
    }
    if let Some(scope) = authed_user
        .get("scope")
        .and_then(Value::as_str)
        .or_else(|| data.get("scope").and_then(Value::as_str))
    {
        value["scope"] = json!(scope);
    }
    atomic_write_token_file(path, value)?;
    Ok(RefreshOutcome::Refreshed)
}

fn atomic_write_token_file(path: &std::path::Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn current_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn render_history(data: Value, channel: &str) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|message| compact_history_message(&message, channel))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "ok": true,
        "messages": messages,
        "next_cursor": data.pointer("/response_metadata/next_cursor").cloned().unwrap_or(Value::Null),
        "has_more": data.get("has_more").cloned().unwrap_or(json!(false))
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn render_users_list(data: Value) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    let members = data
        .get("members")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|user| compact_directory_user(&user))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "ok": true,
        "members": members,
        "next_cursor": data.pointer("/response_metadata/next_cursor").cloned().unwrap_or(Value::Null)
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn render_search_messages(data: Value) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    let matches = data
        .pointer("/results/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|message| compact_search_message(&message))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({"ok": true, "matches": matches})).unwrap_or_else(|_| "{}".into())
}

fn render_search_users(data: Value) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    let users = data
        .pointer("/results/users")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|user| compact_search_user(&user))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({"ok": true, "users": users})).unwrap_or_else(|_| "{}".into())
}

fn render_search_files(data: Value) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    let files = data
        .pointer("/results/files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|file| compact_search_file(&file))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({"ok": true, "files": files})).unwrap_or_else(|_| "{}".into())
}

fn render_user_info(data: Value) -> String {
    if data.get("ok").and_then(Value::as_bool) != Some(true) {
        return serde_json::to_string(&data).unwrap_or_else(|_| "{}".into());
    }
    serde_json::to_string(&json!({
        "ok": true,
        "user": compact_directory_user(data.get("user").unwrap_or(&Value::Null))
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn compact_history_message(message: &Value, channel: &str) -> Value {
    json!({
        "ts": message.get("ts").cloned().unwrap_or(Value::Null),
        "user": message.get("user")
            .or_else(|| message.get("bot_id"))
            .or_else(|| message.get("username"))
            .cloned()
            .unwrap_or(Value::Null),
        "channel": channel,
        "text": truncate(message.get("text").and_then(Value::as_str).unwrap_or("")),
        "thread_ts": message.get("thread_ts").cloned().unwrap_or(Value::Null),
        "reply_count": message.get("reply_count").cloned().unwrap_or(Value::Null),
    })
}

fn compact_search_message(message: &Value) -> Value {
    json!({
        "ts": message.get("message_ts").cloned().unwrap_or(Value::Null),
        "thread_ts": message.get("thread_ts").cloned().unwrap_or(Value::Null),
        "channel": message.get("channel_id").cloned().unwrap_or(Value::Null),
        "channel_name": message.get("channel_name").cloned().unwrap_or(Value::Null),
        "user": message.get("author_user_id").cloned().unwrap_or(Value::Null),
        "user_name": message.get("author_name").cloned().unwrap_or(Value::Null),
        "text": truncate(message.get("content").and_then(Value::as_str).unwrap_or("")),
        "permalink": message.get("permalink").cloned().unwrap_or(Value::Null),
    })
}

fn compact_search_user(user: &Value) -> Value {
    json!({
        "id": user.get("user_id").cloned().unwrap_or(Value::Null),
        "name": user.get("full_name").cloned().unwrap_or(Value::Null),
        "email": user.get("email").cloned().unwrap_or(Value::Null),
        "title": user.get("title").cloned().unwrap_or(Value::Null),
        "tz": user.get("timezone").cloned().unwrap_or(Value::Null),
        "permalink": user.get("permalink").cloned().unwrap_or(Value::Null),
    })
}

fn compact_search_file(file: &Value) -> Value {
    json!({
        "id": file.get("id").or_else(|| file.get("file_id")).cloned().unwrap_or(Value::Null),
        "name": file.get("name").or_else(|| file.get("title")).cloned().unwrap_or(Value::Null),
        "mimetype": file.get("mimetype").cloned().unwrap_or(Value::Null),
        "channel": file.get("channel_id").cloned().unwrap_or(Value::Null),
        "channel_name": file.get("channel_name").cloned().unwrap_or(Value::Null),
        "user": file.get("user_id").or_else(|| file.get("author_user_id")).cloned().unwrap_or(Value::Null),
        "user_name": file.get("author_name").cloned().unwrap_or(Value::Null),
        "ts": file.get("ts").or_else(|| file.get("message_ts")).cloned().unwrap_or(Value::Null),
        "permalink": file.get("permalink").cloned().unwrap_or(Value::Null),
    })
}

fn compact_directory_user(user: &Value) -> Value {
    let profile = user.get("profile").unwrap_or(&Value::Null);
    json!({
        "id": user.get("id").cloned().unwrap_or(Value::Null),
        "name": user.get("name").cloned().unwrap_or(Value::Null),
        "real_name": user.get("real_name").or_else(|| profile.get("real_name")).cloned().unwrap_or(Value::Null),
        "display_name": profile.get("display_name").cloned().unwrap_or(Value::Null),
        "email": profile.get("email").cloned().unwrap_or(Value::Null),
        "title": profile.get("title").cloned().unwrap_or(Value::Null),
        "is_bot": user.get("is_bot").cloned().unwrap_or(json!(false)),
        "deleted": user.get("deleted").cloned().unwrap_or(json!(false)),
        "tz": user.get("tz").cloned().unwrap_or(Value::Null),
    })
}

fn truncate(text: &str) -> String {
    const MAX: usize = 500;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}

fn limit_arg(args: &Value, default: u64) -> u64 {
    args.get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(1, 100)
}

fn push_optional(args: &Value, params: &mut Vec<(String, String)>, key: &str) {
    if let Some(value) = args
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        params.push((key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_default_allows_writes_to_anywhere() {
        let safety = SlackSafety::default();
        assert!(safety.write_allowed_to("U12F7K329"));
        assert!(safety.write_allowed_to("C0123456789"));
        assert!(safety.write_allowed_to("D0AUX99UNR0"));
    }

    #[test]
    fn safety_read_only_blocks_all_writes_even_when_target_listed() {
        let safety = SlackSafety {
            read_only: true,
            write_allow: vec!["U12F7K329".into()],
        };
        assert!(!safety.write_allowed_to("U12F7K329"));
        assert!(!safety.write_allowed_to("anything"));
    }

    #[test]
    fn safety_write_allow_restricts_to_listed_targets() {
        let safety = SlackSafety {
            read_only: false,
            write_allow: vec!["U12F7K329".into(), "C0123456789".into()],
        };
        assert!(safety.write_allowed_to("U12F7K329"));
        assert!(safety.write_allowed_to("C0123456789"));
        assert!(!safety.write_allowed_to("DSomeoneElse"));
        assert!(!safety.write_allowed_to(""));
    }

    #[test]
    fn full_catalog_lists_eleven_tools() {
        let tools = full_tool_catalog();
        assert_eq!(tools.len(), 11);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        for expected in &[
            "slack_whoami",
            "slack_channel_history",
            "slack_search_messages",
            "slack_search_users",
            "slack_search_files",
            "slack_channel_replies",
            "slack_list_users",
            "slack_user_info",
            "slack_permalink",
            "slack_post_as_me",
            "slack_dm_self",
        ] {
            assert!(
                names.contains(expected),
                "missing tool {expected} in catalog: {names:?}"
            );
        }
    }

    #[test]
    fn write_tools_constant_matches_catalog() {
        let catalog = full_tool_catalog();
        let names: std::collections::HashSet<&str> = catalog
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        for write in WRITE_TOOLS {
            assert!(
                names.contains(write),
                "WRITE_TOOLS lists {write} but catalog does not"
            );
        }
    }
}
