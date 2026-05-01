use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use zunel_bus::MessageBus;
use zunel_channels::build_channel_manager;
use zunel_channels::slack::bot_refresh::{
    refresh_bot_if_near_expiry, RefreshContext, RefreshOutcome,
};
use zunel_core::{
    build_default_registry, build_default_registry_async, AgentLoop, ApprovalScope,
    RuntimeSelfStateProvider, SessionManager, SubagentManager,
};
use zunel_tools::{self_tool::SelfTool, spawn::SpawnTool};

use crate::cli::GatewayArgs;
use crate::commands::gateway_scheduler::GatewayScheduler;

/// Wake up every 30 minutes to check if the rotating Slack bot token
/// is within 30 minutes of expiry and refresh it if so. Tunable in
/// tests via `ZUNEL_BOT_REFRESH_TICK_SECS` / `ZUNEL_BOT_REFRESH_WINDOW_SECS`.
const BOT_REFRESH_DEFAULT_TICK_SECS: u64 = 1800;
const BOT_REFRESH_DEFAULT_WINDOW_SECS: i64 = 1800;

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

    let bot_refresh_task = spawn_bot_refresh_task(config_path);

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
    if let Some(handle) = bot_refresh_task {
        handle.abort();
    }
    Ok(())
}

/// Periodic in-runtime Slack bot token refresh.
///
/// Returns `Some(handle)` when a `slack-app/app_info.json` is present
/// (indicating the rotating-bot setup is in use), and `None` otherwise
/// so users without bot rotation pay no cost, and the gateway startup
/// keeps working unchanged. When active, the task wakes every
/// `BOT_REFRESH_DEFAULT_TICK_SECS` seconds and calls
/// `refresh_bot_if_near_expiry(_, Some(BOT_REFRESH_DEFAULT_WINDOW_SECS))`
/// (exactly the code path `zunel slack refresh-bot --if-near-expiry 1800`
/// runs from the LaunchAgent wrapper). With this in place, `brew services
/// start zunel` and `zunel gateway` directly are fully self-sufficient:
/// the external `~/.zunel/bin/run-gateway.sh` and
/// `com.zunel.gateway-rotate` 6h kicker LaunchAgents become optional.
/// Refresh failures are logged at WARN and never crash the gateway,
/// matching the fail-soft policy of the wrapper.
fn spawn_bot_refresh_task(config_path: Option<&Path>) -> Option<tokio::task::JoinHandle<()>> {
    let cfg_path = match config_path {
        Some(path) => path.to_path_buf(),
        None => match zunel_config::default_config_path() {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(error = %err, "in-runtime slack bot refresh disabled: cannot resolve config path");
                return None;
            }
        },
    };
    let home = match zunel_config::zunel_home() {
        Ok(home) => home,
        Err(err) => {
            tracing::warn!(error = %err, "in-runtime slack bot refresh disabled: cannot resolve zunel home");
            return None;
        }
    };
    let app_info = home.join("slack-app").join("app_info.json");
    if !app_info.exists() {
        tracing::debug!(
            path = %app_info.display(),
            "in-runtime slack bot refresh inactive: no rotating-bot app_info on disk"
        );
        return None;
    }

    let tick_secs = parse_env_or("ZUNEL_BOT_REFRESH_TICK_SECS", BOT_REFRESH_DEFAULT_TICK_SECS);
    let window = parse_env_or(
        "ZUNEL_BOT_REFRESH_WINDOW_SECS",
        BOT_REFRESH_DEFAULT_WINDOW_SECS as u64,
    ) as i64;
    let ctx = RefreshContext::from_zunel_home(&home, cfg_path);

    Some(tokio::spawn(refresh_loop(ctx, tick_secs, window, app_info)))
}

async fn refresh_loop(
    ctx: RefreshContext,
    tick_secs: u64,
    window_secs: i64,
    app_info_path: PathBuf,
) {
    tracing::info!(
        path = %app_info_path.display(),
        tick_secs,
        window_secs,
        "starting in-runtime slack bot token refresh"
    );
    let mut ticker = tokio::time::interval(Duration::from_secs(tick_secs));
    // First tick fires immediately and runs the refresh check on
    // startup so the gateway picks up a freshly-rotated token before
    // the inbound loop opens its first Slack websocket.
    loop {
        ticker.tick().await;
        match refresh_bot_if_near_expiry(&ctx, Some(window_secs)).await {
            Ok(RefreshOutcome::Skipped { secs_until_exp, .. }) => {
                tracing::debug!(
                    secs_until_exp,
                    "slack bot token still fresh; skipping refresh"
                );
            }
            Ok(RefreshOutcome::Refreshed {
                expires_at,
                expires_in,
            }) => {
                tracing::info!(
                    expires_at,
                    expires_in,
                    "refreshed slack bot token via in-runtime task"
                );
            }
            Err(err) => {
                tracing::warn!(error = %err, "in-runtime slack bot refresh failed; will retry on next tick");
            }
        }
    }
}

fn parse_env_or(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(default)
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
