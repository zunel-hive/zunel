use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use zunel_tools::{Tool, ToolContext, ToolResult};

use crate::{McpClient, McpToolDefinition};

pub type SharedMcpClient = Arc<Mutex<Box<dyn McpClient>>>;

pub struct McpToolWrapper {
    name: &'static str,
    original_name: String,
    description: &'static str,
    parameters: Value,
    client: SharedMcpClient,
    tool_timeout_secs: u64,
}

impl McpToolWrapper {
    pub fn new(
        server_name: &str,
        definition: McpToolDefinition,
        client: SharedMcpClient,
        tool_timeout_secs: u64,
    ) -> Self {
        let name = leak_string(format!("mcp_{server_name}_{}", definition.name));
        let description = leak_string(
            definition
                .description
                .clone()
                .unwrap_or_else(|| definition.name.clone()),
        );
        Self {
            name,
            original_name: definition.name,
            description,
            parameters: definition.input_schema,
            client,
            tool_timeout_secs,
        }
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        // Forward the agent's depth in the chain so the receiving
        // server can enforce its own --max-call-depth cap. For local
        // / stdio MCP transports the default impl drops the value;
        // only the HTTP transport actually puts the header on the
        // wire (see `RemoteMcpClient::call_tool_with_depth`).
        let outbound_depth = Some(ctx.outbound_call_depth());
        match client
            .call_tool_with_depth(
                &self.original_name,
                args,
                self.tool_timeout_secs,
                outbound_depth,
            )
            .await
        {
            Ok(content) => ToolResult::ok(content),
            Err(err) => ToolResult::err(format!("(MCP tool call failed: {err})")),
        }
    }
}

fn leak_string(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}
