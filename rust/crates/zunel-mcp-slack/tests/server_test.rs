use assert_cmd::cargo::cargo_bin;
use std::collections::BTreeMap;
use std::fs;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_mcp::StdioMcpClient;

#[tokio::test]
async fn native_slack_mcp_server_lists_and_calls_whoami_tool() {
    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "SLACK_USER_TOKEN".to_string(),
        "xoxp-test-token".to_string(),
    );
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "slack_whoami"));

    let result = client
        .call_tool("slack_whoami", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(result.contains("slack token configured"), "{result}");
    assert!(!result.contains("xoxp-test-token"), "{result}");
}

#[tokio::test]
async fn native_slack_mcp_server_reads_user_token_from_zunel_home() {
    let home = tempfile::tempdir().unwrap();
    let token_path = home.path().join("slack-app-mcp").join("user_token.json");
    fs::create_dir_all(token_path.parent().unwrap()).unwrap();
    fs::write(
        &token_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "access_token": "xoxp-file-token",
            "user_id": "UFILE",
            "team_id": "TFILE",
            "team_name": "File Team",
            "scope": "channels:history"
        }))
        .unwrap(),
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [{"ts": "1.0", "user": "UFILE", "text": "from file token"}],
            "has_more": false
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let result = client
        .call_tool(
            "slack_channel_history",
            serde_json::json!({"channel": "C1"}),
            5,
        )
        .await
        .unwrap();
    assert!(result.contains("\"text\":\"from file token\""), "{result}");
    assert!(!result.contains("xoxp-file-token"), "{result}");
}

#[tokio::test]
async fn native_slack_mcp_server_refreshes_expired_user_token_from_app_info() {
    let home = tempfile::tempdir().unwrap();
    let app_dir = home.path().join("slack-app-mcp");
    let token_path = app_dir.join("user_token.json");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("app_info.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "client_id": "111.222",
            "client_secret": "secret"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &token_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "access_token": "xoxp-old-token",
            "refresh_token": "refresh-old",
            "expires_at": 1,
            "user_id": "UOLD",
            "team_id": "TOLD",
            "scope": "users:read"
        }))
        .unwrap(),
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "access_token": "xoxp-new-token",
            "refresh_token": "refresh-new",
            "expires_in": 3600,
            "scope": "users:read",
            "token_type": "user",
            "authed_user": {"id": "UNEW"},
            "team": {"id": "TNEW", "name": "Team New"},
            "enterprise": {"id": "ENEW"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.list"))
        .and(header("authorization", "Bearer xoxp-new-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "members": [{"id": "UNEW", "name": "new", "profile": {"real_name": "New User"}}],
            "response_metadata": {"next_cursor": ""}
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let users = client
        .call_tool("slack_list_users", serde_json::json!({"limit": 2}), 5)
        .await
        .unwrap();
    assert!(users.contains("\"id\":\"UNEW\""), "{users}");

    let token: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&token_path).unwrap()).unwrap();
    assert_eq!(token["access_token"], "xoxp-new-token");
    assert_eq!(token["refresh_token"], "refresh-new");
    assert_eq!(token["user_id"], "UNEW");
    assert_eq!(token["team_id"], "TNEW");
    assert!(token["expires_at"].as_i64().unwrap() > 1);
}

#[tokio::test]
async fn native_slack_mcp_server_surfaces_refresh_failure_when_oauth_v2_access_rejects() {
    let home = tempfile::tempdir().unwrap();
    let app_dir = home.path().join("slack-app-mcp");
    let token_path = app_dir.join("user_token.json");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("app_info.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "client_id": "111.222",
            "client_secret": "secret"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &token_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "access_token": "xoxp-stale-token",
            "refresh_token": "refresh-revoked",
            "expires_at": 1,
            "user_id": "USTALE",
            "team_id": "TSTALE",
            "scope": "chat:write"
        }))
        .unwrap(),
    )
    .unwrap();

    let server = MockServer::start().await;
    // Slack rejects the cached refresh_token (e.g. it aged out beyond the
    // rotation window or was revoked) — the recovery path should surface
    // the underlying `invalid_refresh_token` and the remediation hint
    // instead of the bare `token_expired` from chat.postMessage.
    Mock::given(method("POST"))
        .and(path("/api/oauth.v2.access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": false,
            "error": "invalid_refresh_token"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": false,
            "error": "token_expired"
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let result = client
        .call_tool(
            "slack_post_as_me",
            serde_json::json!({"channel": "C1", "text": "hi"}),
            5,
        )
        .await
        .unwrap();
    assert!(
        result.contains("invalid_refresh_token"),
        "expected refresh error to be surfaced, got: {result}"
    );
    assert!(
        result.contains("zunel slack login --force"),
        "expected remediation hint, got: {result}"
    );
    assert!(
        !result.contains("\"error\":\"token_expired\""),
        "bare `token_expired` should have been replaced, got: {result}"
    );
    // The cached file should not have been overwritten with garbage.
    let token: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&token_path).unwrap()).unwrap();
    assert_eq!(token["access_token"], "xoxp-stale-token");
    assert_eq!(token["refresh_token"], "refresh-revoked");
}

#[tokio::test]
async fn native_slack_mcp_server_posts_as_user_and_dm_self() {
    let home = tempfile::tempdir().unwrap();
    let token_path = home.path().join("slack-app-mcp").join("user_token.json");
    fs::create_dir_all(token_path.parent().unwrap()).unwrap();
    fs::write(
        &token_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "access_token": "xoxp-file-token",
            "user_id": "UFILE",
            "team_id": "TFILE",
            "team_name": "File Team"
        }))
        .unwrap(),
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channel": "C1",
            "ts": "1713974400.000100"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.getPermalink"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "permalink": "https://slack.example/msg"
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "slack_post_as_me"));
    assert!(tools.iter().any(|tool| tool.name == "slack_dm_self"));

    let empty = client
        .call_tool(
            "slack_post_as_me",
            serde_json::json!({"channel": "C1", "text": "   "}),
            5,
        )
        .await
        .unwrap();
    assert!(empty.contains("\"error\":\"empty_text\""), "{empty}");

    let posted = client
        .call_tool(
            "slack_post_as_me",
            serde_json::json!({"channel": "C1", "text": "hello"}),
            5,
        )
        .await
        .unwrap();
    assert!(posted.contains("\"ok\":true"), "{posted}");
    assert!(
        posted.contains("\"permalink\":\"https://slack.example/msg\""),
        "{posted}"
    );

    let dm = client
        .call_tool("slack_dm_self", serde_json::json!({"text": "note"}), 5)
        .await
        .unwrap();
    assert!(dm.contains("\"ok\":true"), "{dm}");
}

#[tokio::test]
async fn native_slack_mcp_server_searches_messages_users_and_files() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/assistant.search.context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "results": {
                "messages": [{
                    "message_ts": "1713974400.000100",
                    "thread_ts": "1713974400.000100",
                    "channel_id": "C1",
                    "channel_name": "general",
                    "author_user_id": "U1",
                    "author_name": "Ray",
                    "content": "launch notes",
                    "permalink": "https://slack.example/msg"
                }],
                "users": [{
                    "user_id": "U1",
                    "full_name": "Ray User",
                    "email": "ray@example.com",
                    "title": "Engineer",
                    "timezone": "America/Los_Angeles",
                    "permalink": "https://slack.example/user"
                }],
                "files": [{
                    "file_id": "F1",
                    "title": "Plan",
                    "mimetype": "text/plain",
                    "channel_id": "C1",
                    "channel_name": "general",
                    "author_user_id": "U1",
                    "author_name": "Ray",
                    "message_ts": "1713974400.000100",
                    "permalink": "https://slack.example/file"
                }]
            }
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "SLACK_USER_TOKEN".to_string(),
        "xoxp-test-token".to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool.name == "slack_search_messages"));
    assert!(tools.iter().any(|tool| tool.name == "slack_search_users"));
    assert!(tools.iter().any(|tool| tool.name == "slack_search_files"));

    let messages = client
        .call_tool(
            "slack_search_messages",
            serde_json::json!({"query": "launch", "limit": 3}),
            5,
        )
        .await
        .unwrap();
    assert!(messages.contains("\"matches\""), "{messages}");
    assert!(messages.contains("\"text\":\"launch notes\""), "{messages}");

    let users = client
        .call_tool(
            "slack_search_users",
            serde_json::json!({"query": "ray", "limit": 3}),
            5,
        )
        .await
        .unwrap();
    assert!(users.contains("\"users\""), "{users}");
    assert!(users.contains("\"email\":\"ray@example.com\""), "{users}");

    let files = client
        .call_tool(
            "slack_search_files",
            serde_json::json!({"query": "plan", "limit": 3}),
            5,
        )
        .await
        .unwrap();
    assert!(files.contains("\"files\""), "{files}");
    assert!(files.contains("\"id\":\"F1\""), "{files}");
}

#[tokio::test]
async fn native_slack_mcp_server_reads_channel_history_and_user_info() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [{
                "ts": "1713974400.000100",
                "user": "U1",
                "text": "hello from slack",
                "thread_ts": "1713974400.000100",
                "reply_count": 2
            }],
            "has_more": false,
            "response_metadata": {"next_cursor": ""}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1",
                "name": "ray",
                "real_name": "Ray",
                "profile": {"display_name": "Ray", "email": "ray@example.com", "title": "Engineer"},
                "is_bot": false,
                "deleted": false,
                "tz": "America/Los_Angeles"
            }
        })))
        .mount(&server)
        .await;

    let bin = cargo_bin("zunel-mcp-slack");
    let mut env = BTreeMap::new();
    env.insert(
        "SLACK_USER_TOKEN".to_string(),
        "xoxp-test-token".to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), server.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool.name == "slack_channel_history"));
    assert!(tools.iter().any(|tool| tool.name == "slack_user_info"));

    let history = client
        .call_tool(
            "slack_channel_history",
            serde_json::json!({"channel": "C1", "limit": 5}),
            5,
        )
        .await
        .unwrap();
    assert!(history.contains("\"ok\":true"), "{history}");
    assert!(history.contains("\"channel\":\"C1\""), "{history}");
    assert!(
        history.contains("\"text\":\"hello from slack\""),
        "{history}"
    );

    let user = client
        .call_tool("slack_user_info", serde_json::json!({"user": "U1"}), 5)
        .await
        .unwrap();
    assert!(user.contains("\"ok\":true"), "{user}");
    assert!(user.contains("\"email\":\"ray@example.com\""), "{user}");
    assert!(user.contains("\"title\":\"Engineer\""), "{user}");
}
