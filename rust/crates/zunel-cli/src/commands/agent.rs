use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use zunel_core::{
    build_default_registry, AgentLoop, ApprovalHandler, ApprovalScope, SessionManager,
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

fn parse_approval_scope(s: &str) -> ApprovalScope {
    match s.to_ascii_lowercase().as_str() {
        "shell" => ApprovalScope::Shell,
        "writes" | "write" => ApprovalScope::Writes,
        // "all", empty string, "none", anything else collapses to All;
        // gating happens at `approval_required = false` for the off case.
        _ => ApprovalScope::All,
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
