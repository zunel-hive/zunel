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

/// Wake up every 30 minutes to walk OAuth-enabled remote MCP servers
/// and rotate any access token whose refresh leeway is up. Tunable
/// via `ZUNEL_MCP_REFRESH_TICK_SECS`; set `ZUNEL_MCP_REFRESH_DISABLED=1`
/// to opt out entirely.
const MCP_REFRESH_DEFAULT_TICK_SECS: u64 = 1800;

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
    let mcp_refresh_task = spawn_mcp_refresh_task(config_path);

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
    if let Some(handle) = mcp_refresh_task {
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

/// Periodic in-runtime refresh of every OAuth-enabled remote MCP
/// server's cached access token.
///
/// Mirrors [`spawn_bot_refresh_task`] (commit 4a34100) but for the
/// MCP side: every `ZUNEL_MCP_REFRESH_TICK_SECS` seconds (default
/// 30 min) the task walks each OAuth-enabled remote server in the
/// loaded config and calls `zunel_mcp::oauth::refresh_if_needed`
/// against its cached token. Tokens within the leeway window are
/// rotated via `grant_type=refresh_token` and rewritten atomically;
/// `RemoteMcpClient`'s live auth-provider closure picks up the new
/// bearer on the very next request, so long-running gateways
/// (notably `brew services start zunel`) never serve a stale 401
/// to Slack-driven MCP calls.
///
/// Returns `None` when:
/// - `ZUNEL_MCP_REFRESH_DISABLED` is set to anything truthy
///   (`1`, `true`, …) — operators who want full external control.
/// - The config can't be loaded — we surface a warn and back off so
///   gateway startup keeps succeeding.
/// - The active config has no OAuth-enabled remote MCP servers — we
///   wouldn't have anything to refresh.
fn spawn_mcp_refresh_task(config_path: Option<&Path>) -> Option<tokio::task::JoinHandle<()>> {
    if env_disabled("ZUNEL_MCP_REFRESH_DISABLED") {
        tracing::info!("in-runtime MCP OAuth refresh disabled via ZUNEL_MCP_REFRESH_DISABLED");
        return None;
    }
    let cfg = match zunel_config::load_config(config_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::warn!(error = %err, "in-runtime MCP OAuth refresh disabled: cannot load config");
            return None;
        }
    };
    let oauth_servers: Vec<String> = cfg
        .tools
        .mcp_servers
        .iter()
        .filter(|(_, server)| {
            server.url.is_some()
                && server
                    .normalized_oauth()
                    .map(|oauth| oauth.enabled)
                    .unwrap_or(false)
        })
        .map(|(name, _)| name.clone())
        .collect();
    if oauth_servers.is_empty() {
        tracing::debug!(
            "in-runtime MCP OAuth refresh inactive: no OAuth-enabled remote MCP servers configured"
        );
        return None;
    }
    let home = match zunel_config::zunel_home() {
        Ok(home) => home,
        Err(err) => {
            tracing::warn!(error = %err, "in-runtime MCP OAuth refresh disabled: cannot resolve zunel home");
            return None;
        }
    };
    let tick_secs = parse_env_or("ZUNEL_MCP_REFRESH_TICK_SECS", MCP_REFRESH_DEFAULT_TICK_SECS);
    Some(tokio::spawn(mcp_refresh_loop(
        cfg,
        home,
        tick_secs,
        oauth_servers,
    )))
}

async fn mcp_refresh_loop(
    cfg: zunel_config::Config,
    home: PathBuf,
    tick_secs: u64,
    server_names: Vec<String>,
) {
    tracing::info!(
        servers = ?server_names,
        tick_secs,
        "starting in-runtime MCP OAuth token refresh"
    );
    let mut ticker = tokio::time::interval(Duration::from_secs(tick_secs));
    // First tick fires immediately and re-validates every cached
    // token on startup, in addition to the per-server validation
    // `register_mcp_tools` already did. The duplication is cheap
    // (one disk read per server when the token's still fresh) and
    // catches the corner case where the gateway's been up across a
    // refresh-token revocation that happened while it was idle.
    loop {
        ticker.tick().await;
        let outcomes = zunel_mcp::oauth::refresh_all_oauth_servers(&home, &cfg).await;
        for (server, outcome) in outcomes {
            log_mcp_refresh_outcome(&server, outcome);
        }
    }
}

fn log_mcp_refresh_outcome(server: &str, outcome: zunel_mcp::OAuthRefreshOutcome) {
    use zunel_mcp::OAuthRefreshOutcome::*;
    match outcome {
        StillFresh { secs_remaining } => {
            tracing::debug!(
                server,
                secs_remaining,
                "MCP OAuth token still fresh; refresh tick skipped"
            );
        }
        Refreshed { new_expires_in } => {
            tracing::info!(
                server,
                new_expires_in,
                "refreshed MCP OAuth access token via in-runtime task"
            );
        }
        NotCached | NoExpiry => {}
        NoRefreshToken => {
            tracing::warn!(
                server,
                "MCP OAuth refresh tick: no refresh_token cached; user must re-login \
                 (chat: ask the agent; CLI: `zunel mcp login {server} --force`)",
                server = server
            );
        }
        NoTokenUrl => {
            tracing::warn!(
                server,
                "MCP OAuth refresh tick: token cache is missing tokenUrl; user must re-login \
                 (chat: ask the agent; CLI: `zunel mcp login {server} --force`)",
                server = server
            );
        }
        RefreshFailed(err) => {
            tracing::warn!(
                server,
                error = %err,
                "MCP OAuth refresh tick failed; will retry on next tick"
            );
        }
    }
}

fn env_disabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
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

    // Load skills from `<workspace>/skills/` plus the binary-bundled
    // builtins (e.g. `mcp-oauth-login`). User skills win on name
    // collisions; embedded builtins fill in otherwise.
    let skills = zunel_skills::SkillsLoader::new(&workspace, None, &[]);

    Ok(
        AgentLoop::with_sessions(provider, cfg.agents.defaults.clone(), sessions)
            .with_tools(registry)
            .with_workspace(workspace)
            .with_skills(skills)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zunel_config::mcp_oauth::{load_token, save_token, CachedMcpOAuthToken};

    #[test]
    fn env_disabled_recognises_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "YES"] {
            std::env::set_var("ZUNEL_TEST_ENV_DISABLED_FLAG", value);
            assert!(
                env_disabled("ZUNEL_TEST_ENV_DISABLED_FLAG"),
                "{value} should be truthy"
            );
        }
        for value in ["0", "false", "no", "", "off"] {
            std::env::set_var("ZUNEL_TEST_ENV_DISABLED_FLAG", value);
            assert!(
                !env_disabled("ZUNEL_TEST_ENV_DISABLED_FLAG"),
                "{value} should NOT be truthy"
            );
        }
        std::env::remove_var("ZUNEL_TEST_ENV_DISABLED_FLAG");
        assert!(!env_disabled("ZUNEL_TEST_ENV_DISABLED_FLAG"));
    }

    /// One tick of the periodic refresh task: configure an
    /// OAuth-enabled remote MCP server with a stale cached token,
    /// run a single iteration of [`refresh_all_oauth_servers`] (the
    /// inner step the loop performs), and assert the on-disk cache
    /// was rewritten. Cheap and deterministic — no `interval()`
    /// pause juggling needed because the loop's tick body is one
    /// straight-line library call.
    #[tokio::test]
    async fn mcp_refresh_tick_rewrites_stale_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "fresh-access",
                "refresh_token": "fresh-refresh",
                "token_type": "Bearer",
                "expires_in": 7200
            })))
            .mount(&server)
            .await;

        let home = tempfile::tempdir().unwrap();
        let stale = CachedMcpOAuthToken {
            access_token: "stale-access".into(),
            token_type: Some("Bearer".into()),
            refresh_token: Some("stale-refresh".into()),
            expires_in: Some(60),
            scope: None,
            obtained_at: 1,
            client_id: "client".into(),
            client_secret: None,
            authorization_url: format!("{}/authorize", server.uri()),
            token_url: format!("{}/token", server.uri()),
        };
        save_token(home.path(), "remote", &stale).unwrap();

        let raw = json!({
            "providers": {},
            "agents": {"defaults": {"model": "m"}},
            "tools": {"mcpServers": {"remote": {
                "type": "streamableHttp",
                "url": format!("{}/mcp", server.uri()),
                "oauth": {"enabled": true}
            }}}
        });
        let cfg: zunel_config::Config = serde_json::from_value(raw).unwrap();

        let outcomes = zunel_mcp::oauth::refresh_all_oauth_servers(home.path(), &cfg).await;
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0].1,
            zunel_mcp::OAuthRefreshOutcome::Refreshed { .. }
        ));

        let after = load_token(home.path(), "remote").unwrap().unwrap();
        assert_eq!(after.access_token, "fresh-access");
        assert_eq!(after.refresh_token.as_deref(), Some("fresh-refresh"));
        assert_eq!(after.expires_in, Some(7200));
    }
}
