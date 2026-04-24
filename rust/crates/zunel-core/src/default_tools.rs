//! Slice 3 default tool seeding.
//!
//! Builds a `ToolRegistry` populated with the standard zunel toolset
//! based on a [`Config`]. Read-only filesystem and search tools are
//! always seeded; `exec` and the web tools are gated behind their
//! respective `enable` flags to match Python's parity behavior.

use std::path::Path;
use std::sync::Arc;

use zunel_config::{Config, WebToolsConfig};
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
