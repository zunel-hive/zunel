use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use zunel_tools::{Tool, ToolContext, ToolResult};

use crate::stdio::{McpToolDefinition, StdioMcpClient};

pub struct McpToolWrapper {
    name: &'static str,
    original_name: String,
    description: &'static str,
    parameters: Value,
    client: Arc<Mutex<StdioMcpClient>>,
    tool_timeout_secs: u64,
}

impl McpToolWrapper {
    pub fn new(
        server_name: &str,
        definition: McpToolDefinition,
        client: Arc<Mutex<StdioMcpClient>>,
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

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        match client
            .call_tool(&self.original_name, args, self.tool_timeout_secs)
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
