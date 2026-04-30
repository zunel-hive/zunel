use assert_cmd::cargo::cargo_bin;
use std::collections::BTreeMap;
use std::fs;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_mcp::StdioMcpClient;

#[tokio::test]
async fn native_self_mcp_server_lists_and_calls_status_tool() {
    let bin = cargo_bin("zunel-mcp-self");
    let mut client =
        StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], BTreeMap::new(), 5)
            .await
            .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "self_status"));

    let result = client
        .call_tool("self_status", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(result.contains("zunel-self ok"), "{result}");
}

#[tokio::test]
async fn native_self_mcp_server_lists_sessions_from_active_workspace() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("workspace");
    let sessions = workspace.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("cli_direct.jsonl"),
        r#"{"_type": "metadata", "key": "cli:direct", "created_at": "2026-04-24T10:00:00.000000", "updated_at": "2026-04-24T11:00:00.000000", "metadata": {}, "last_consolidated": 0}
{"role": "user", "content": "hello", "timestamp": "2026-04-24T11:00:00.000000"}
"#,
    )
    .unwrap();
    fs::write(
        home.path().join("config.json"),
        format!(
            r#"{{
                "providers": {{}},
                "agents": {{"defaults": {{"model": "m", "workspace": "{}"}}}}
            }}"#,
            workspace.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    let bin = cargo_bin("zunel-mcp-self");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "zunel_sessions_list"));

    let result = client
        .call_tool("zunel_sessions_list", serde_json::json!({"limit": 5}), 5)
        .await
        .unwrap();
    assert!(result.contains("\"count\":1"), "{result}");
    assert!(result.contains("\"key\":\"cli:direct\""), "{result}");
    assert!(result.contains("\"message_count\":1"), "{result}");
}

#[tokio::test]
async fn native_self_mcp_server_gets_session_metadata_and_messages() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("workspace");
    let sessions = workspace.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("cli_direct.jsonl"),
        r#"{"_type": "metadata", "key": "cli:direct", "created_at": "2026-04-24T10:00:00.000000", "updated_at": "2026-04-24T11:00:00.000000", "metadata": {"source": "test"}, "last_consolidated": 0}
{"role": "user", "content": "hello", "timestamp": "2026-04-24T11:00:00.000000"}
{"role": "assistant", "content": "hi", "timestamp": "2026-04-24T11:00:01.000000"}
"#,
    )
    .unwrap();
    fs::write(
        home.path().join("config.json"),
        format!(
            r#"{{
                "providers": {{}},
                "agents": {{"defaults": {{"model": "m", "workspace": "{}"}}}}
            }}"#,
            workspace.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    let bin = cargo_bin("zunel-mcp-self");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "zunel_session_get"));
    assert!(tools
        .iter()
        .any(|tool| tool.name == "zunel_session_messages"));

    let metadata = client
        .call_tool(
            "zunel_session_get",
            serde_json::json!({"session_key": "cli:direct"}),
            5,
        )
        .await
        .unwrap();
    assert!(metadata.contains("\"found\":true"), "{metadata}");
    assert!(metadata.contains("\"message_count\":2"), "{metadata}");
    assert!(metadata.contains("\"source\":\"test\""), "{metadata}");

    let messages = client
        .call_tool(
            "zunel_session_messages",
            serde_json::json!({"session_key": "cli:direct", "limit": 1}),
            5,
        )
        .await
        .unwrap();
    assert!(messages.contains("\"count\":1"), "{messages}");
    assert!(messages.contains("\"content\":\"hi\""), "{messages}");
    assert!(!messages.contains("\"content\":\"hello\""), "{messages}");
}

#[tokio::test]
async fn native_self_mcp_server_lists_channels_mcp_servers_and_cron_jobs() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("workspace");
    fs::create_dir_all(workspace.join("cron")).unwrap();
    fs::write(
        home.path().join("config.json"),
        format!(
            r#"{{
                "providers": {{}},
                "agents": {{"defaults": {{"model": "m", "workspace": "{}"}}}},
                "channels": {{
                    "slack": {{
                        "enabled": true,
                        "mode": "socket",
                        "botToken": "xoxb-secret",
                        "allowFrom": ["U1"]
                    }}
                }},
                "tools": {{
                    "mcpServers": {{
                        "local": {{
                            "type": "stdio",
                            "command": "zunel-mcp-self",
                            "args": ["--flag"],
                            "enabledTools": ["self_status"],
                            "env": {{"SECRET": "hidden"}}
                        }}
                    }}
                }}
            }}"#,
            workspace.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    fs::write(
        workspace.join("cron").join("jobs.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "jobs": [{
                "id": "job_1",
                "name": "daily",
                "enabled": true,
                "schedule": {"kind": "every", "everyMs": 60000},
                "payload": {"kind": "agent_turn", "message": "tick", "deliver": false},
                "state": {"nextRunAtMs": 1000, "runHistory": []},
                "createdAtMs": 1,
                "updatedAtMs": 1,
                "deleteAfterRun": false
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let bin = cargo_bin("zunel-mcp-self");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "zunel_channels_list"));
    assert!(tools
        .iter()
        .any(|tool| tool.name == "zunel_mcp_servers_list"));
    assert!(tools.iter().any(|tool| tool.name == "zunel_cron_jobs_list"));
    assert!(tools.iter().any(|tool| tool.name == "zunel_cron_job_get"));

    let channels = client
        .call_tool("zunel_channels_list", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(channels.contains("\"name\":\"slack\""), "{channels}");
    assert!(channels.contains("\"enabled\":true"), "{channels}");
    assert!(!channels.contains("xoxb-secret"), "{channels}");

    let servers = client
        .call_tool("zunel_mcp_servers_list", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(servers.contains("\"name\":\"local\""), "{servers}");
    assert!(
        servers.contains("\"command\":\"zunel-mcp-self\""),
        "{servers}"
    );
    assert!(!servers.contains("hidden"), "{servers}");

    let jobs = client
        .call_tool("zunel_cron_jobs_list", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(jobs.contains("\"id\":\"job_1\""), "{jobs}");

    let job = client
        .call_tool(
            "zunel_cron_job_get",
            serde_json::json!({"job_id": "job_1"}),
            5,
        )
        .await
        .unwrap();
    assert!(job.contains("\"found\":true"), "{job}");
    assert!(job.contains("\"message\":\"tick\""), "{job}");
}

#[tokio::test]
async fn native_self_mcp_server_reports_token_usage() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("workspace");
    let sessions = workspace.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("cli_direct.jsonl"),
        r#"{"_type": "metadata", "key": "cli:direct", "created_at": "2026-04-24T10:00:00.000000", "updated_at": "2026-04-24T11:00:00.000000", "metadata": {"usage_total": {"prompt_tokens": 1200, "completion_tokens": 300, "reasoning_tokens": 50, "cached_tokens": 0, "turns": 5}, "turn_usage": [{"ts": "2026-04-24T11:00:00.000000", "prompt": 1200, "completion": 300, "reasoning": 50, "cached": 0}]}, "last_consolidated": 0}
"#,
    )
    .unwrap();
    fs::write(
        home.path().join("config.json"),
        format!(
            r#"{{
                "providers": {{}},
                "agents": {{"defaults": {{"model": "m", "workspace": "{}"}}}}
            }}"#,
            workspace.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    let bin = cargo_bin("zunel-mcp-self");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "zunel_token_usage"));

    let total = client
        .call_tool("zunel_token_usage", serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(total.contains("\"prompt_tokens\":1200"), "{total}");
    assert!(total.contains("\"completion_tokens\":300"), "{total}");
    assert!(total.contains("\"reasoning_tokens\":50"), "{total}");
    assert!(total.contains("\"sessions\":1"), "{total}");

    let scoped = client
        .call_tool(
            "zunel_token_usage",
            serde_json::json!({"session_key": "cli:direct"}),
            5,
        )
        .await
        .unwrap();
    assert!(scoped.contains("\"key\":\"cli:direct\""), "{scoped}");
    assert!(scoped.contains("\"turns\":5"), "{scoped}");
}

#[tokio::test]
async fn native_self_mcp_server_sends_slack_message_to_channel() {
    let home = tempfile::tempdir().unwrap();
    let workspace = home.path().join("workspace");
    let slack = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channel": "C1",
            "ts": "1713974400.000100"
        })))
        .mount(&slack)
        .await;

    fs::write(
        home.path().join("config.json"),
        format!(
            r#"{{
                "providers": {{}},
                "agents": {{"defaults": {{"model": "m", "workspace": "{}"}}}},
                "channels": {{"slack": {{"enabled": true, "botToken": "xoxb-secret"}}}}
            }}"#,
            workspace.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    let bin = cargo_bin("zunel-mcp-self");
    let mut env = BTreeMap::new();
    env.insert(
        "ZUNEL_HOME".to_string(),
        home.path().to_string_lossy().to_string(),
    );
    env.insert("SLACK_API_BASE".to_string(), slack.uri());
    let mut client = StdioMcpClient::connect(bin.to_string_lossy().as_ref(), &[], env, 5)
        .await
        .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool.name == "zunel_send_message_to_channel"));

    let unsupported = client
        .call_tool(
            "zunel_send_message_to_channel",
            serde_json::json!({"channel": "email", "channel_id": "C1", "text": "hello"}),
            5,
        )
        .await
        .unwrap();
    assert!(unsupported.contains("\"ok\":false"), "{unsupported}");

    let sent = client
        .call_tool(
            "zunel_send_message_to_channel",
            serde_json::json!({"channel": "slack", "channel_id": "C1", "text": "hello"}),
            5,
        )
        .await
        .unwrap();
    assert!(sent.contains("\"ok\":true"), "{sent}");
    assert!(sent.contains("\"channel\":\"C1\""), "{sent}");
    assert!(!sent.contains("xoxb-secret"), "{sent}");
}
