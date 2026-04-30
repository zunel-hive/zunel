use wiremock::matchers::{body_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_bus::{MessageKind, OutboundMessage};
use zunel_channels::slack::SlackChannel;
use zunel_channels::Channel;
use zunel_config::SlackChannelConfig;

#[tokio::test]
async fn slack_channel_sends_final_message_via_chat_post_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(header("authorization", "Bearer xoxb-test"))
        .and(body_json(serde_json::json!({
            "channel": "C123",
            "text": "hello slack",
            "thread_ts": "T456"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "ts": "123.456"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.remove"))
        .and(header("authorization", "Bearer xoxb-test"))
        .and(body_json(serde_json::json!({
            "channel": "C123",
            "name": "eyes",
            "timestamp": "T456"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.add"))
        .and(header("authorization", "Bearer xoxb-test"))
        .and(body_json(serde_json::json!({
            "channel": "C123",
            "name": "white_check_mark",
            "timestamp": "T456"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let channel = SlackChannel::new(SlackChannelConfig {
        enabled: true,
        bot_token: Some("xoxb-test".into()),
        app_token: Some("xapp-test".into()),
        reply_in_thread: true,
        ..Default::default()
    })
    .with_api_base(server.uri());

    channel
        .send(OutboundMessage {
            channel: "slack".into(),
            chat_id: "C123:T456".into(),
            message_id: None,
            content: "hello slack".into(),
            media: Vec::new(),
            kind: MessageKind::Final,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn slack_channel_sends_approval_message_with_buttons() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(header("authorization", "Bearer xoxb-test"))
        .and(body_string_contains("\"channel\":\"C123\""))
        .and(body_string_contains("\"text\":\"Approval required\""))
        .and(body_string_contains("\"thread_ts\":\"T456\""))
        .and(body_string_contains("\"action_id\":\"zunel_approve_once\""))
        .and(body_string_contains("\"action_id\":\"zunel_approve_deny\""))
        .and(body_string_contains("slack:C123:T456"))
        .and(body_string_contains("req-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "ts": "123.456"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let channel = SlackChannel::new(SlackChannelConfig {
        enabled: true,
        bot_token: Some("xoxb-test".into()),
        app_token: Some("xapp-test".into()),
        reply_in_thread: true,
        ..Default::default()
    })
    .with_api_base(server.uri());

    channel
        .send(OutboundMessage {
            channel: "slack".into(),
            chat_id: "C123:T456".into(),
            message_id: Some("req-1".into()),
            content: "Approval required".into(),
            media: Vec::new(),
            kind: MessageKind::Approval,
        })
        .await
        .unwrap();
}
