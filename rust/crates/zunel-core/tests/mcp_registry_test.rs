use std::fs;

use serde_json::json;
use tempfile::tempdir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_config::{
    AgentDefaults, AgentsConfig, Config, McpServerConfig, ProvidersConfig, ToolsConfig,
};
use zunel_core::build_default_registry_async;
use zunel_tools::ToolContext;

fn fixture_server_script() -> String {
    r#"
import json
import sys

def read_msg():
    header = b""
    while b"\r\n\r\n" not in header:
        chunk = sys.stdin.buffer.read(1)
        if not chunk:
            raise SystemExit(0)
        header += chunk
    length = 0
    for line in header.decode("utf-8").split("\r\n"):
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1].strip())
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))

def send(obj):
    body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

while True:
    msg = read_msg()
    method = msg.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "fixture", "version": "1"}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"message": "listing"}})
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": [{"name": "echo", "description": "Echo text", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}, {"name": "bad name", "description": "Invalid name", "inputSchema": {"type": "object", "properties": {}}}]}})
    elif method == "tools/call":
        text = msg.get("params", {}).get("arguments", {}).get("text", "")
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": "echo:" + text}]}})
"#
    .to_string()
}

#[tokio::test]
async fn async_default_registry_loads_stdio_mcp_tools_after_native_tools() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("fixture_mcp.py");
    fs::write(&script, fixture_server_script()).unwrap();

    let mut cfg = Config {
        providers: ProvidersConfig::default(),
        agents: AgentsConfig {
            defaults: AgentDefaults {
                model: "m".into(),
                ..Default::default()
            },
        },
        channels: Default::default(),
        gateway: Default::default(),
        tools: ToolsConfig::default(),
        cli: Default::default(),
    };
    cfg.tools.mcp_servers.insert(
        "fixture".into(),
        McpServerConfig {
            transport_type: Some("stdio".into()),
            command: Some("python3".into()),
            args: Some(vec![script.to_string_lossy().to_string()]),
            tool_timeout: Some(5),
            init_timeout: Some(5),
            enabled_tools: Some(vec!["echo".into()]),
            ..Default::default()
        },
    );

    let registry = build_default_registry_async(&cfg, dir.path()).await;
    assert!(registry.get("cron").is_some());
    assert!(registry.get("mcp_fixture_echo").is_some());
    assert!(registry.get("mcp_fixture_bad name").is_none());

    let definitions = registry.get_definitions();
    let names: Vec<&str> = definitions
        .iter()
        .map(|value| value["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.last(), Some(&"mcp_fixture_echo"));

    let result = registry
        .execute(
            "mcp_fixture_echo",
            json!({"text": "hello"}),
            &ToolContext::for_test(),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "echo:hello");
}

#[tokio::test]
async fn async_default_registry_loads_streamable_http_mcp_tools() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_string_contains("\"method\":\"initialize\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"protocolVersion": "2024-11-05", "capabilities": {}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_string_contains("notifications/initialized"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_string_contains("\"method\":\"tools/list\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [{
                    "name": "remote_echo",
                    "description": "Remote echo",
                    "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
                }]
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_string_contains("\"method\":\"tools/call\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {"content": [{"type": "text", "text": "remote:hello"}]}
        })))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let mut cfg = Config {
        providers: ProvidersConfig::default(),
        agents: AgentsConfig {
            defaults: AgentDefaults {
                model: "m".into(),
                ..Default::default()
            },
        },
        channels: Default::default(),
        gateway: Default::default(),
        tools: ToolsConfig::default(),
        cli: Default::default(),
    };
    cfg.tools.mcp_servers.insert(
        "remote".into(),
        McpServerConfig {
            transport_type: Some("streamableHttp".into()),
            url: Some(format!("{}/mcp", server.uri())),
            tool_timeout: Some(5),
            init_timeout: Some(5),
            enabled_tools: Some(vec!["*".into()]),
            ..Default::default()
        },
    );

    let registry = build_default_registry_async(&cfg, dir.path()).await;
    assert!(registry.get("mcp_remote_remote_echo").is_some());
    let result = registry
        .execute(
            "mcp_remote_remote_echo",
            json!({"text": "hello"}),
            &ToolContext::for_test(),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "remote:hello");
}
