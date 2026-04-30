//! Slack Web API primitives + media download helpers.
//!
//! Everything in this module is a thin wrapper around a single Slack HTTP
//! call so the [`super`] driver can stay focused on lifecycle (connect, run
//! socket loop, dispatch events). Each helper takes `client` /  `api_base` /
//! `bot_token` (or `app_token`) explicitly so the same primitives are usable
//! from the `start()` socket task without owning a `&self`.

use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use zunel_bus::{MessageKind, OutboundMessage};
use zunel_config::SlackChannelConfig;

use crate::{Error, Result};

/// `auth.test` — returns the bot user id (when present) so the inbound loop
/// can suppress its own messages.
pub(super) async fn auth_test(
    client: &reqwest::Client,
    api_base: &str,
    bot_token: &str,
) -> Result<Option<String>> {
    let mut auth =
        HeaderValue::from_str(&format!("Bearer {bot_token}")).map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: format!("invalid bot token header: {e}"),
        })?;
    auth.set_sensitive(true);
    let response = client
        .post(format!("{api_base}/api/auth.test"))
        .header(AUTHORIZATION, auth)
        .send()
        .await
        .map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: e.to_string(),
        })?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|e| Error::Channel {
        channel: "slack".into(),
        message: e.to_string(),
    })?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(Error::Channel {
            channel: "slack".into(),
            message: format!(
                "auth.test failed: {}",
                payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_error")
            ),
        });
    }
    Ok(payload
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|user_id| !user_id.is_empty())
        .map(str::to_string))
}

/// `apps.connections.open` — exchange the app-level token for a Socket Mode
/// WebSocket URL (each URL is single-use).
pub(super) async fn open_socket_url(
    client: &reqwest::Client,
    api_base: &str,
    app_token: &str,
) -> Result<String> {
    let mut auth =
        HeaderValue::from_str(&format!("Bearer {app_token}")).map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: format!("invalid app token header: {e}"),
        })?;
    auth.set_sensitive(true);
    let response = client
        .post(format!("{api_base}/api/apps.connections.open"))
        .header(AUTHORIZATION, auth)
        .send()
        .await
        .map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: e.to_string(),
        })?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|e| Error::Channel {
        channel: "slack".into(),
        message: e.to_string(),
    })?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(Error::Channel {
            channel: "slack".into(),
            message: payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("apps.connections.open failed")
                .to_string(),
        });
    }
    payload
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Channel {
            channel: "slack".into(),
            message: "apps.connections.open returned no url".into(),
        })
}

/// Generic `reactions.add` / `reactions.remove`.
pub(super) async fn post_reaction(
    client: &reqwest::Client,
    api_base: &str,
    bot_token: &str,
    method: &str,
    channel: &str,
    name: &str,
    timestamp: &str,
) -> Result<()> {
    let mut auth =
        HeaderValue::from_str(&format!("Bearer {bot_token}")).map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: format!("invalid bot token header: {e}"),
        })?;
    auth.set_sensitive(true);
    let response = client
        .post(format!("{api_base}/api/{method}"))
        .header(AUTHORIZATION, auth)
        .json(&json!({
            "channel": channel,
            "name": name,
            "timestamp": timestamp
        }))
        .send()
        .await
        .map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: e.to_string(),
        })?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|e| Error::Channel {
        channel: "slack".into(),
        message: e.to_string(),
    })?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(Error::Channel {
            channel: "slack".into(),
            message: payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("reaction failed")
                .to_string(),
        });
    }
    Ok(())
}

/// `chat.postMessage` for an outbound zunel message. Handles the two
/// approval-button / done-emoji side-effects so the `Channel::send` impl can
/// stay a one-liner.
pub(super) async fn send_outbound(
    client: &reqwest::Client,
    api_base: &str,
    config: &SlackChannelConfig,
    bot_token: &str,
    message: &OutboundMessage,
) -> Result<()> {
    let (channel_id, thread_ts) = slack_target(&message.chat_id);
    let mut body = json!({
        "channel": channel_id,
        "text": message.content,
    });
    if message.kind == MessageKind::Approval {
        if let Some(request_id) = message.message_id.as_deref() {
            body["blocks"] = approval_blocks(&message.content, &message.chat_id, request_id);
        }
    }
    if config.reply_in_thread {
        if let Some(thread_ts) = thread_ts {
            body["thread_ts"] = json!(thread_ts);
        }
    }
    let mut auth =
        HeaderValue::from_str(&format!("Bearer {bot_token}")).map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: format!("invalid bot token header: {e}"),
        })?;
    auth.set_sensitive(true);
    let response = client
        .post(format!("{api_base}/api/chat.postMessage"))
        .header(AUTHORIZATION, auth)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Channel {
            channel: "slack".into(),
            message: e.to_string(),
        })?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|e| Error::Channel {
        channel: "slack".into(),
        message: e.to_string(),
    })?;
    if !status.is_success() || payload.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(Error::Channel {
            channel: "slack".into(),
            message: payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("chat.postMessage failed")
                .to_string(),
        });
    }
    if message.kind == MessageKind::Final {
        if let Some(thread_ts) = thread_ts {
            if let Some(emoji) = config.react_emoji.as_deref() {
                let _ = post_reaction(
                    client,
                    api_base,
                    bot_token,
                    "reactions.remove",
                    channel_id,
                    emoji,
                    thread_ts,
                )
                .await;
            }
            if let Some(emoji) = config.done_emoji.as_deref() {
                let _ = post_reaction(
                    client,
                    api_base,
                    bot_token,
                    "reactions.add",
                    channel_id,
                    emoji,
                    thread_ts,
                )
                .await;
            }
        }
    }
    Ok(())
}

/// Download every Slack file attached to an inbound event into
/// `~/.zunel/media/` (or `$TMPDIR/zunel-media/` when home is unavailable),
/// returning the on-disk paths so the agent loop can attach them.
pub(super) async fn download_slack_files(
    client: &reqwest::Client,
    bot_token: &str,
    value: &Value,
) -> Vec<String> {
    let Some(files) = value
        .get("payload")
        .and_then(|payload| payload.get("event"))
        .and_then(|event| event.get("files"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let media_dir = match zunel_config::zunel_home() {
        Ok(home) => home.join("media"),
        Err(_) => std::env::temp_dir().join("zunel-media"),
    };
    if tokio::fs::create_dir_all(&media_dir).await.is_err() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    for file in files {
        let Some(url) = file
            .get("url_private_download")
            .or_else(|| file.get("url_private"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let name = file
            .get("name")
            .or_else(|| file.get("id"))
            .and_then(Value::as_str)
            .map(sanitize_filename)
            .unwrap_or_else(|| "slack-file".into());
        let path = media_dir.join(name);
        let mut auth = match HeaderValue::from_str(&format!("Bearer {bot_token}")).map_err(|e| {
            Error::Channel {
                channel: "slack".into(),
                message: format!("invalid bot token header: {e}"),
            }
        }) {
            Ok(auth) => auth,
            Err(_) => continue,
        };
        auth.set_sensitive(true);
        let Ok(response) = client.get(url).header(AUTHORIZATION, auth).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else {
            continue;
        };
        if tokio::fs::write(&path, bytes).await.is_ok() {
            paths.push(path.display().to_string());
        }
    }
    paths
}

/// Split a zunel `chat_id` of the form `<channel>:<thread_ts>` into its
/// channel and (optional) thread-timestamp parts. Plain `chat_id` values
/// without a `:` are returned as `(chat_id, None)`.
pub(super) fn slack_target(chat_id: &str) -> (&str, Option<&str>) {
    match chat_id.split_once(':') {
        Some((channel, thread_ts)) if !thread_ts.is_empty() => (channel, Some(thread_ts)),
        _ => (chat_id, None),
    }
}

fn sanitize_filename(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "slack-file".into()
    } else {
        sanitized
    }
}

fn approval_blocks(content: &str, session_key: &str, request_id: &str) -> Value {
    let approve_value = json!({
        "session_key": format!("slack:{session_key}"),
        "request_id": request_id
    })
    .to_string();
    let deny_value = json!({
        "session_key": format!("slack:{session_key}"),
        "request_id": request_id
    })
    .to_string();
    json!([
        {
            "type": "section",
            "text": {"type": "mrkdwn", "text": content}
        },
        {
            "type": "actions",
            "elements": [
                {
                    "type": "button",
                    "text": {"type": "plain_text", "text": "Approve"},
                    "style": "primary",
                    "action_id": "zunel_approve_once",
                    "value": approve_value
                },
                {
                    "type": "button",
                    "text": {"type": "plain_text", "text": "Deny"},
                    "style": "danger",
                    "action_id": "zunel_approve_deny",
                    "value": deny_value
                }
            ]
        }
    ])
}
