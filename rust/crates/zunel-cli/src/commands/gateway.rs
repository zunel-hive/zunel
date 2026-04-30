use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use zunel_bus::MessageBus;
use zunel_channels::build_channel_manager;
use zunel_core::{
    build_default_registry, build_default_registry_async, AgentLoop, ApprovalScope,
    RuntimeSelfStateProvider, SessionManager, SubagentManager,
};
use zunel_tools::{self_tool::SelfTool, spawn::SpawnTool};

use crate::cli::GatewayArgs;
use crate::commands::gateway_scheduler::GatewayScheduler;

pub async fn run(args: GatewayArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path).with_context(|| "loading zunel config")?;
    let workspace = zunel_config::workspace_path(&cfg.agents.defaults)
        .with_context(|| "resolving workspace path")?;
    zunel_config::guard_workspace(&workspace).with_context(|| "validating workspace path")?;
    zunel_util::ensure_dir(&workspace)
        .with_context(|| format!("creating workspace dir {}", workspace.display()))?;

    let bus = Arc::new(MessageBus::new(256));
    let channels = build_channel_manager(&cfg.channels, bus.clone());

    if args.dry_run {
        let statuses = channels.statuses().await;
        println!(
            "gateway ready: workspace={}, channels: {}",
            workspace.display(),
            statuses.len()
        );
        return Ok(());
    }

    channels
        .start_all()
        .await
        .with_context(|| "starting channels")?;
    let statuses = channels.statuses().await;
    println!(
        "gateway started: workspace={}, channels: {}",
        workspace.display(),
        statuses.len()
    );

    if args.startup_only {
        channels
            .stop_all()
            .await
            .with_context(|| "stopping channels")?;
        return Ok(());
    }

    let agent_loop = Arc::new(build_gateway_agent_loop(&cfg, workspace.clone()).await?);

    if let Some(max_inbound) = args.max_inbound {
        for _ in 0..max_inbound {
            agent_loop
                .process_inbound_once(&bus)
                .await
                .with_context(|| "processing inbound gateway message")?;
            channels
                .dispatch_next_outbound()
                .await
                .with_context(|| "dispatching outbound gateway message")?;
        }
        println!("gateway processed inbound: {max_inbound}");
        channels
            .stop_all()
            .await
            .with_context(|| "stopping channels")?;
        return Ok(());
    }

    let inbound_bus = bus.clone();
    let inbound_loop = agent_loop.clone();
    let inbound_task = tokio::spawn(async move {
        loop {
            if let Err(err) = inbound_loop.process_inbound_once(&inbound_bus).await {
                tracing::warn!("gateway inbound processing failed: {err}");
            }
        }
    });

    let dispatch_channels = channels.clone();
    let dispatch_task = tokio::spawn(async move {
        loop {
            match dispatch_channels.dispatch_next_outbound().await {
                Ok(true) => {}
                Ok(false) => break,
                Err(err) => tracing::warn!("gateway outbound dispatch failed: {err}"),
            }
        }
    });

    let scheduler_task = match build_scheduler(&cfg, workspace.clone()) {
        Ok(scheduler) => Some(scheduler.spawn()),
        Err(err) => {
            tracing::warn!(error = %err, "gateway scheduler disabled");
            None
        }
    };

    tokio::signal::ctrl_c()
        .await
        .with_context(|| "waiting for shutdown signal")?;
    channels
        .stop_all()
        .await
        .with_context(|| "stopping channels")?;
    inbound_task.abort();
    dispatch_task.abort();
    if let Some(handle) = scheduler_task {
        handle.abort();
    }
    Ok(())
}

fn build_scheduler(
    cfg: &zunel_config::Config,
    workspace: std::path::PathBuf,
) -> Result<GatewayScheduler> {
    let provider =
        zunel_providers::build_provider(cfg).with_context(|| "building scheduler provider")?;
    GatewayScheduler::from_config(cfg, workspace, provider)
}

async fn build_gateway_agent_loop(
    cfg: &zunel_config::Config,
    workspace: std::path::PathBuf,
) -> Result<AgentLoop> {
    let provider = zunel_providers::build_provider(cfg).with_context(|| "building provider")?;
    let sessions = SessionManager::new(&workspace);
    let mut registry = build_default_registry_async(cfg, &workspace).await;
    let child_tools = build_default_registry(cfg, &workspace);
    let subagents = Arc::new(
        SubagentManager::new(
            provider.clone(),
            workspace.clone(),
            cfg.agents.defaults.model.clone(),
        )
        .with_child_tools(child_tools),
    );
    registry.register(Arc::new(SpawnTool::new(subagents.clone())));
    let mut tool_names: Vec<String> = registry.names().map(str::to_string).collect();
    tool_names.push("self".into());
    registry.register(Arc::new(SelfTool::from_provider(Arc::new(
        RuntimeSelfStateProvider {
            model: cfg.agents.defaults.model.clone(),
            provider: cfg
                .agents
                .defaults
                .provider
                .clone()
                .unwrap_or_else(|| "custom".into()),
            workspace: workspace.display().to_string(),
            max_iterations: cfg
                .agents
                .defaults
                .max_tool_iterations
                .unwrap_or(15)
                .try_into()
                .unwrap_or(u32::MAX),
            tools: tool_names,
            subagents,
        },
    ))));

    Ok(
        AgentLoop::with_sessions(provider, cfg.agents.defaults.clone(), sessions)
            .with_tools(registry)
            .with_workspace(workspace)
            .with_approval_required(cfg.tools.approval_required)
            .with_approval_scope(parse_approval_scope(&cfg.tools.approval_scope))
            .with_show_token_footer(cfg.channels.show_token_footer),
    )
}

fn parse_approval_scope(s: &str) -> ApprovalScope {
    match s.to_ascii_lowercase().as_str() {
        "shell" => ApprovalScope::Shell,
        "writes" | "write" => ApprovalScope::Writes,
        _ => ApprovalScope::All,
    }
}
