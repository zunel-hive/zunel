//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod loader;
mod paths;
mod schema;

pub use error::{Error, Result};
pub use loader::load_config;
pub use paths::{
    cli_history_path, default_config_path, default_workspace_path, sessions_dir, workspace_path,
    zunel_home,
};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ExecToolsConfig,
    FilesystemToolsConfig, McpOAuthConfig, McpServerConfig, ProvidersConfig, ToolsConfig,
    WebToolsConfig,
};
