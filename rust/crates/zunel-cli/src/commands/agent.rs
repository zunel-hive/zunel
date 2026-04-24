use std::path::Path;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use zunel_core::{AgentLoop, SessionManager};

use crate::cli::AgentArgs;
use crate::renderer::StreamingRenderer;

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path)
        .with_context(|| "loading zunel config")?;
    let workspace = zunel_config::workspace_path(&cfg.agents.defaults)
        .with_context(|| "resolving workspace path")?;
    zunel_util::ensure_dir(&workspace).with_context(|| {
        format!("creating workspace dir {}", workspace.display())
    })?;

    let provider = zunel_providers::build_provider(&cfg)
        .with_context(|| "building provider")?;
    let sessions = SessionManager::new(&workspace);
    let agent_loop = AgentLoop::with_sessions(provider, cfg.agents.defaults.clone(), sessions);

    let (tx, rx) = mpsc::channel(64);
    let renderer = StreamingRenderer::start();
    let render_task = tokio::spawn(async move { renderer.drive(rx).await });

    let session_key = "cli:direct";
    let _result = agent_loop
        .process_streamed(session_key, &args.message, tx)
        .await
        .with_context(|| "running agent")?;

    // Let the renderer finish consuming.
    render_task.await.map_err(|e| anyhow::anyhow!("render task failed: {e}"))??;

    Ok(())
}
