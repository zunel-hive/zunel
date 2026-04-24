use std::path::Path;

use anyhow::{Context, Result};
use zunel_core::AgentLoop;

use crate::cli::AgentArgs;

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path).with_context(|| "loading zunel config")?;
    let provider = zunel_providers::build_provider(&cfg).with_context(|| "building provider")?;
    let agent_loop = AgentLoop::new(provider, cfg.agents.defaults);
    let result = agent_loop
        .process_direct(&args.message)
        .await
        .with_context(|| "running agent")?;
    println!("{}", result.content);
    Ok(())
}
