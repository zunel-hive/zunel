//! Slice 3 default tool seeding.
//!
//! Builds a `ToolRegistry` populated with the standard zunel toolset
//! based on a [`Config`]. Read-only filesystem and search tools are
//! always seeded; `exec` and the web tools are gated behind their
//! respective `enable` flags to match Python's parity behavior.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use zunel_config::{Config, McpServerConfig, WebToolsConfig};
use zunel_mcp::{McpToolWrapper, StdioMcpClient};
use zunel_tools::{
    fs::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool},
    path_policy::PathPolicy,
    search::{GlobTool, GrepTool},
    shell::ExecTool,
    web::{WebFetchTool, WebSearchTool},
    BraveProvider, DuckDuckGoProvider, StubProvider, ToolRegistry, WebSearchProvider,
};

pub fn build_default_registry(cfg: &Config, workspace: &Path) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    let mut policy = PathPolicy::restricted(workspace);
    if let Some(media_dir) = cfg.tools.filesystem.media_dir.as_deref() {
        policy = policy.with_media_dir(media_dir);
    }
    registry.register(Arc::new(ReadFileTool::new(policy.clone())));
    registry.register(Arc::new(WriteFileTool::new(policy.clone())));
    registry.register(Arc::new(EditFileTool::new(policy.clone())));
    registry.register(Arc::new(ListDirTool::new(policy.clone())));
    registry.register(Arc::new(GlobTool::new(policy.clone())));
    registry.register(Arc::new(GrepTool::new(policy)));

    if cfg.tools.exec.enable {
        registry.register(Arc::new(ExecTool::new_default()));
    }
    if cfg.tools.web.enable {
        registry.register(Arc::new(WebFetchTool::new()));
        let provider = build_search_provider(&cfg.tools.web);
        registry.register(Arc::new(WebSearchTool::new(provider)));
    }
    registry
}

pub async fn build_default_registry_async(cfg: &Config, workspace: &Path) -> ToolRegistry {
    let mut registry = build_default_registry(cfg, workspace);
    register_mcp_tools(&mut registry, cfg).await;
    registry
}

async fn register_mcp_tools(registry: &mut ToolRegistry, cfg: &Config) {
    for (server_name, server) in &cfg.tools.mcp_servers {
        if !is_stdio_server(server) {
            tracing::warn!(server = %server_name, "skipping non-stdio MCP server");
            continue;
        }
        let Some(command) = server.command.as_deref() else {
            tracing::warn!(server = %server_name, "skipping MCP server without command");
            continue;
        };
        let args = server.args.clone().unwrap_or_default();
        let env = server.env.clone().unwrap_or_default();
        let init_timeout = server.init_timeout.unwrap_or(10);
        let tool_timeout = server.tool_timeout.unwrap_or(30);
        let mut client = match StdioMcpClient::connect(command, &args, env, init_timeout).await {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(server = %server_name, error = %err, "failed to initialize MCP server");
                continue;
            }
        };
        let tools = match client.list_tools(tool_timeout).await {
            Ok(tools) => tools,
            Err(err) => {
                tracing::warn!(server = %server_name, error = %err, "failed to list MCP tools");
                continue;
            }
        };
        let client = Arc::new(Mutex::new(client));
        for tool in tools {
            let wrapped_name = format!("mcp_{server_name}_{}", tool.name);
            if !tool_enabled(server, &tool.name, &wrapped_name) {
                continue;
            }
            registry.register(Arc::new(McpToolWrapper::new(
                server_name,
                tool,
                Arc::clone(&client),
                tool_timeout,
            )));
        }
    }
}

fn is_stdio_server(server: &McpServerConfig) -> bool {
    matches!(server.transport_type.as_deref().unwrap_or("stdio"), "stdio")
}

fn tool_enabled(server: &McpServerConfig, raw_name: &str, wrapped_name: &str) -> bool {
    let Some(enabled) = &server.enabled_tools else {
        return true;
    };
    enabled
        .iter()
        .any(|name| name == raw_name || name == wrapped_name)
}

fn build_search_provider(cfg: &WebToolsConfig) -> Box<dyn WebSearchProvider> {
    match cfg.search_provider.as_str() {
        "brave" => {
            let key = cfg.brave_api_key.clone().unwrap_or_default();
            Box::new(BraveProvider::new(key))
        }
        "duckduckgo" | "ddg" => Box::new(DuckDuckGoProvider::new()),
        // Empty string + anything unknown collapses to a stub provider
        // that returns a clear "unimplemented" error at call time.
        _ => Box::new(StubProvider {
            provider_name: "stub",
        }),
    }
}
