use std::collections::BTreeMap;
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{timeout, Duration};

use crate::schema::normalize_schema_for_openai;
use crate::{Error, McpClient, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

pub struct StdioMcpClient {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: u64,
}

impl StdioMcpClient {
    pub async fn connect(
        command: &str,
        args: &[String],
        env: BTreeMap<String, String>,
        init_timeout_secs: u64,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("stdio MCP child missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("stdio MCP child missing stdout".into()))?;
        let mut client = Self {
            _child: child,
            stdin,
            stdout,
            next_id: 1,
        };
        timeout(
            Duration::from_secs(init_timeout_secs),
            client.request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "zunel-rust", "version": env!("CARGO_PKG_VERSION")}
                }),
            ),
        )
        .await
        .map_err(|_| Error::Timeout(init_timeout_secs))??;
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    pub async fn list_tools(&mut self, timeout_secs: u64) -> Result<Vec<McpToolDefinition>> {
        let response = timeout(
            Duration::from_secs(timeout_secs),
            self.request("tools/list", json!({})),
        )
        .await
        .map_err(|_| {
            self.kill_child();
            Error::Timeout(timeout_secs)
        })??;
        let tools = response
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Protocol("tools/list response missing tools array".into()))?;
        tools
            .iter()
            .map(|tool| {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Protocol("MCP tool missing name".into()))?;
                Ok(McpToolDefinition {
                    name: name.to_string(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    input_schema: normalize_schema_for_openai(
                        tool.get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    ),
                })
            })
            .collect()
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout_secs: u64,
    ) -> Result<String> {
        let response = timeout(
            Duration::from_secs(timeout_secs),
            self.request("tools/call", json!({"name": name, "arguments": arguments})),
        )
        .await
        .map_err(|_| {
            self.kill_child();
            Error::Timeout(timeout_secs)
        })??;
        Ok(render_call_result(&response))
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        write_frame(&mut self.stdin, &request).await?;
        loop {
            let response = read_frame(&mut self.stdout).await?;
            let response_id = response.get("id").and_then(Value::as_u64);
            if response_id.is_none() {
                continue;
            }
            if response_id != Some(id) {
                tracing::debug!(
                    method,
                    expected = id,
                    got = ?response_id,
                    "ignoring MCP response for a different request"
                );
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(Error::Protocol(format!("MCP {method} failed: {error}")));
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| Error::Protocol(format!("MCP {method} response missing result")));
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        write_frame(&mut self.stdin, &request).await
    }

    fn kill_child(&mut self) {
        let _ = self._child.start_kill();
    }
}

#[async_trait::async_trait]
impl McpClient for StdioMcpClient {
    async fn list_tools(&mut self, timeout_secs: u64) -> Result<Vec<McpToolDefinition>> {
        StdioMcpClient::list_tools(self, timeout_secs).await
    }

    async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout_secs: u64,
    ) -> Result<String> {
        StdioMcpClient::call_tool(self, name, arguments, timeout_secs).await
    }
}

async fn write_frame(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_frame(stdout: &mut ChildStdout) -> Result<Value> {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = stdout.read(&mut byte).await?;
        if n == 0 {
            return Err(Error::Protocol("MCP child closed stdout".into()));
        }
        header.push(byte[0]);
        if header.len() > 8192 {
            return Err(Error::Protocol("MCP frame header too large".into()));
        }
    }
    let header = String::from_utf8(header)
        .map_err(|e| Error::Protocol(format!("MCP frame header is not UTF-8: {e}")))?;
    let content_length = header
        .split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| Error::Protocol("MCP frame missing Content-Length".into()))?;
    let mut body = vec![0_u8; content_length];
    stdout.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub(crate) fn render_call_result(value: &Value) -> String {
    let Some(content) = value.get("content").and_then(Value::as_array) else {
        return value.to_string();
    };
    let parts: Vec<String> = content
        .iter()
        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
            Some("text") => item
                .get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        value.to_string()
    } else {
        parts.join("\n")
    }
}
