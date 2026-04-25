use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use serde_json::json;
use tempfile::tempdir;
use tokio::sync::Mutex;
use zunel_mcp::{McpToolWrapper, StdioMcpClient};
use zunel_tools::{Tool, ToolContext};

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
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": [{"name": "echo", "description": "Echo text", "inputSchema": {"type": "object", "properties": {"text": {"type": ["string", "null"]}}}}]}})
    elif method == "tools/call":
        text = msg.get("params", {}).get("arguments", {}).get("text", "")
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": "echo:" + text}]}})
    else:
        send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "unknown method"}})
"#
    .to_string()
}

#[tokio::test]
async fn stdio_client_lists_and_calls_fixture_tool() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("fixture_mcp.py");
    fs::write(&script, fixture_server_script()).unwrap();

    let mut client = StdioMcpClient::connect(
        "python3",
        &[script.to_string_lossy().to_string()],
        BTreeMap::new(),
        5,
    )
    .await
    .unwrap();

    let tools = client.list_tools(5).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert_eq!(
        tools[0].input_schema["properties"]["text"]["nullable"],
        true
    );

    let output = client
        .call_tool("echo", json!({"text": "hello"}), 5)
        .await
        .unwrap();
    assert_eq!(output, "echo:hello");
}

#[tokio::test]
async fn wrapper_exposes_mcp_tool_as_zunel_tool() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("fixture_mcp.py");
    fs::write(&script, fixture_server_script()).unwrap();

    let mut client = StdioMcpClient::connect(
        "python3",
        &[script.to_string_lossy().to_string()],
        BTreeMap::new(),
        5,
    )
    .await
    .unwrap();
    let tool_def = client.list_tools(5).await.unwrap().remove(0);
    let wrapper = McpToolWrapper::new("fixture", tool_def, Arc::new(Mutex::new(client)), 5);

    assert_eq!(wrapper.name(), "mcp_fixture_echo");
    assert_eq!(wrapper.description(), "Echo text");
    assert_eq!(wrapper.parameters()["properties"]["text"]["nullable"], true);

    let result = wrapper
        .execute(json!({"text": "wrapped"}), &ToolContext::for_test())
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "echo:wrapped");
}
