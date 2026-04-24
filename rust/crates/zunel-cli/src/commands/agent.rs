use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use zunel_config::Config;
use zunel_core::{AgentLoop, ApprovalHandler, ApprovalScope, SessionManager};
use zunel_tools::{
    fs::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool},
    path_policy::PathPolicy,
    search::{GlobTool, GrepTool},
    shell::ExecTool,
    web::{WebFetchTool, WebSearchTool},
    BraveProvider, DuckDuckGoProvider, StubProvider, ToolRegistry, WebSearchProvider,
};

use crate::approval_cli::StdinApprovalHandler;
use crate::cli::AgentArgs;
use crate::renderer::StreamingRenderer;
use crate::repl::{run_repl, ReplConfig};

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path).with_context(|| "loading zunel config")?;
    let workspace = zunel_config::workspace_path(&cfg.agents.defaults)
        .with_context(|| "resolving workspace path")?;
    zunel_util::ensure_dir(&workspace)
        .with_context(|| format!("creating workspace dir {}", workspace.display()))?;

    let provider = zunel_providers::build_provider(&cfg).with_context(|| "building provider")?;
    let sessions = SessionManager::new(&workspace);
    let registry = build_default_registry(&cfg, &workspace);
    let mut builder =
        AgentLoop::with_sessions(provider, cfg.agents.defaults.clone(), sessions.clone())
            .with_tools(registry)
            .with_workspace(workspace.clone())
            .with_approval_required(cfg.tools.approval_required)
            .with_approval_scope(parse_approval_scope(&cfg.tools.approval_scope));
    if cfg.tools.approval_required {
        let handler: Arc<dyn ApprovalHandler> = Arc::new(StdinApprovalHandler::new());
        builder = builder.with_approval(handler);
    }
    let agent_loop = Arc::new(builder);

    match args.message {
        Some(msg) => run_once(agent_loop.as_ref(), &args.session, &msg).await,
        None => {
            let repl_cfg = ReplConfig {
                session_key: args.session.clone(),
                model_label: cfg.agents.defaults.model.clone(),
            };
            run_repl(agent_loop, Arc::new(sessions), repl_cfg).await
        }
    }
}

/// Seed a `ToolRegistry` with the default Slice 3 toolset:
///
/// - read-only filesystem and search tools always on
///   (`read_file`, `list_dir`, `glob`, `grep`)
/// - mutating filesystem tools (`write_file`, `edit_file`) always on
///   inside the workspace sandbox so the agent can act
/// - `exec` gated on `cfg.tools.exec.enable`
/// - `web_fetch` + `web_search` gated on `cfg.tools.web.enable`
///   (provider chosen via `cfg.tools.web.search_provider`)
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

fn parse_approval_scope(s: &str) -> ApprovalScope {
    match s.to_ascii_lowercase().as_str() {
        "shell" => ApprovalScope::Shell,
        "writes" | "write" => ApprovalScope::Writes,
        // "all", empty string, "none", anything else collapses to All;
        // gating happens at `approval_required = false` for the off case.
        _ => ApprovalScope::All,
    }
}

fn build_search_provider(cfg: &zunel_config::WebToolsConfig) -> Box<dyn WebSearchProvider> {
    match cfg.search_provider.as_str() {
        "brave" => {
            let key = cfg.brave_api_key.clone().unwrap_or_default();
            Box::new(BraveProvider::new(key))
        }
        "duckduckgo" | "ddg" => Box::new(DuckDuckGoProvider::new()),
        // Empty string + anything unknown collapses to a stub provider
        // that returns a clear "unimplemented" error at call time so
        // the agent can recover instead of crashing.
        _ => Box::new(StubProvider {
            provider_name: "stub",
        }),
    }
}

async fn run_once(agent_loop: &AgentLoop, session_key: &str, message: &str) -> Result<()> {
    let (tx, rx) = mpsc::channel(64);
    let renderer = StreamingRenderer::start();
    let render_task = tokio::spawn(async move { renderer.drive(rx).await });
    agent_loop
        .process_streamed(session_key, message, tx)
        .await
        .with_context(|| "running agent")?;
    render_task
        .await
        .map_err(|e| anyhow::anyhow!("render task failed: {e}"))??;
    Ok(())
}
