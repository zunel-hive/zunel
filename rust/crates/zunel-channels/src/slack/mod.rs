//! Slack channel implementation.
//!
//! Split into focused submodules:
//!
//! * [`api`] — REST primitives (`auth.test`, `apps.connections.open`,
//!   `chat.postMessage`, `reactions.{add,remove}`, file download).
//! * [`inbound`] — Socket Mode envelope → [`InboundMessage`] parsing
//!   plus all allow/policy/mention rules and tests.
//!
//! This file keeps the [`SlackChannel`] type, its [`Channel`] trait
//! implementation, and the long-running Socket Mode reconnect loop. The
//! loop intentionally lives inline so it can close over the lock-protected
//! `connected` flag without a separate plumbing struct.

mod api;
pub mod bot_refresh;
mod inbound;

use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use zunel_bus::{MessageBus, MessageKind, OutboundMessage};
use zunel_config::SlackChannelConfig;

use crate::{Channel, ChannelStatus, Error, Result};

pub struct SlackChannel {
    config: SlackChannelConfig,
    api_base: String,
    client: reqwest::Client,
    connected: Arc<Mutex<bool>>,
    socket_task: Mutex<Option<JoinHandle<()>>>,
}

impl SlackChannel {
    pub fn new(config: SlackChannelConfig) -> Self {
        Self {
            config,
            api_base: "https://slack.com".into(),
            client: reqwest::Client::new(),
            connected: Arc::new(Mutex::new(false)),
            socket_task: Mutex::new(None),
        }
    }

    pub fn with_api_base(mut self, api_base: String) -> Self {
        self.api_base = api_base.trim_end_matches('/').to_string();
        self
    }

    pub async fn status(&self) -> ChannelStatus {
        self.build_status().await
    }

    async fn build_status(&self) -> ChannelStatus {
        if !self.config.enabled {
            return ChannelStatus {
                name: "slack".into(),
                enabled: false,
                connected: false,
                detail: Some("disabled".into()),
            };
        }

        let missing: Vec<&str> = [
            ("bot token", self.config.bot_token.as_deref()),
            ("app token", self.config.app_token.as_deref()),
        ]
        .into_iter()
        .filter_map(|(label, value)| {
            value
                .filter(|s| !s.is_empty())
                .map(|_| ())
                .is_none()
                .then_some(label)
        })
        .collect();

        if !missing.is_empty() {
            return ChannelStatus {
                name: "slack".into(),
                enabled: true,
                connected: false,
                detail: Some(format!("missing {}", missing.join(" and "))),
            };
        }

        ChannelStatus {
            name: "slack".into(),
            enabled: true,
            connected: *self.connected.lock().await,
            detail: Some("socket mode configured".into()),
        }
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &'static str {
        "slack"
    }

    async fn start(&self, bus: Arc<MessageBus>) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        let status = self.build_status().await;
        if status
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("missing "))
        {
            return Err(Error::Channel {
                channel: "slack".into(),
                message: status.detail.unwrap_or_else(|| "invalid config".into()),
            });
        }
        let app_token = self
            .config
            .app_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Channel {
                channel: "slack".into(),
                message: "missing app token".into(),
            })?;
        let bot_token = self
            .config
            .bot_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Channel {
                channel: "slack".into(),
                message: "missing bot token".into(),
            })?;
        let bot_user_id = api::auth_test(&self.client, &self.api_base, bot_token).await?;
        let socket_url = api::open_socket_url(&self.client, &self.api_base, app_token).await?;
        let (socket, _) = tokio_tungstenite::connect_async(&socket_url)
            .await
            .map_err(|e| Error::Channel {
                channel: "slack".into(),
                message: format!("socket mode connect failed: {e}"),
            })?;
        let first_socket = socket.split();
        let config = self.config.clone();
        let bot_user_id = bot_user_id.clone();
        let connected = self.connected.clone();
        let client = self.client.clone();
        let api_base = self.api_base.clone();
        let bot_token = bot_token.to_string();
        let app_token = app_token.to_string();
        *connected.lock().await = true;
        let task = tokio::spawn(socket_loop(
            first_socket,
            config,
            bot_user_id,
            connected,
            client,
            api_base,
            bot_token,
            app_token,
            bus,
        ));
        *self.socket_task.lock().await = Some(task);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        if let Some(task) = self.socket_task.lock().await.take() {
            task.abort();
        }
        *self.connected.lock().await = false;
        Ok(())
    }

    async fn send(&self, message: OutboundMessage) -> Result<()> {
        if !self.config.enabled {
            return Err(Error::Channel {
                channel: "slack".into(),
                message: "disabled".into(),
            });
        }
        let token = self
            .config
            .bot_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Channel {
                channel: "slack".into(),
                message: "missing bot token".into(),
            })?;
        api::send_outbound(&self.client, &self.api_base, &self.config, token, &message).await
    }

    async fn status(&self) -> ChannelStatus {
        self.build_status().await
    }
}

type SocketHalves = (
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
);

/// Long-running Socket Mode loop. Owns the WS halves, runs forever (until
/// the spawned task is aborted by `stop()`), and silently reconnects on any
/// IO/transport failure with a 250ms backoff.
#[allow(clippy::too_many_arguments)]
async fn socket_loop(
    first_socket: SocketHalves,
    config: SlackChannelConfig,
    bot_user_id: Option<String>,
    connected: Arc<Mutex<bool>>,
    client: reqwest::Client,
    api_base: String,
    bot_token: String,
    app_token: String,
    bus: Arc<MessageBus>,
) {
    let mut first_socket = Some(first_socket);
    loop {
        let (mut write, mut read) = if let Some(socket) = first_socket.take() {
            socket
        } else {
            *connected.lock().await = false;
            tokio::time::sleep(Duration::from_millis(250)).await;
            let Ok(socket_url) = api::open_socket_url(&client, &api_base, &app_token).await else {
                continue;
            };
            let Ok((socket, _)) = tokio_tungstenite::connect_async(&socket_url).await else {
                continue;
            };
            *connected.lock().await = true;
            socket.split()
        };
        while let Some(next) = read.next().await {
            let Ok(message) = next else {
                break;
            };
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(envelope_id) = value.get("envelope_id").and_then(Value::as_str) {
                let _ = write
                    .send(Message::Text(
                        json!({"envelope_id": envelope_id}).to_string().into(),
                    ))
                    .await;
            }
            if let Some(mut inbound) = inbound::socket_interactive_to_inbound(&config, &value)
                .or_else(|| {
                    inbound::socket_message_to_inbound(&config, bot_user_id.as_deref(), &value)
                })
            {
                if inbound.kind == MessageKind::User {
                    if let Some((channel, timestamp, emoji)) =
                        inbound::inbound_reaction_target(&config, &value)
                    {
                        let _ = api::post_reaction(
                            &client,
                            &api_base,
                            &bot_token,
                            "reactions.add",
                            &channel,
                            &emoji,
                            &timestamp,
                        )
                        .await;
                    }
                    inbound.media = api::download_slack_files(&client, &bot_token, &value).await;
                }
                let _ = bus.publish_inbound(inbound).await;
            }
        }
    }
}
