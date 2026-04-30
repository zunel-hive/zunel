use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::OnboardArgs;

pub async fn run(args: OnboardArgs) -> Result<()> {
    let home = zunel_config::zunel_home().with_context(|| "resolving zunel home")?;
    let config_path =
        zunel_config::default_config_path().with_context(|| "resolving config path")?;
    let workspace =
        zunel_config::default_workspace_path().with_context(|| "resolving workspace")?;
    zunel_config::guard_workspace(&workspace).with_context(|| "validating workspace path")?;
    zunel_util::ensure_dir(&home).with_context(|| format!("creating {}", home.display()))?;
    zunel_util::ensure_dir(&workspace)
        .with_context(|| format!("creating {}", workspace.display()))?;
    zunel_util::ensure_dir(&workspace.join("memory"))
        .with_context(|| format!("creating {}", workspace.join("memory").display()))?;

    if args.force || !config_path.exists() {
        let config = json!({
            "providers": {},
            "agents": {
                "defaults": {
                    "provider": "custom",
                    "model": "gpt-4o-mini",
                    "workspace": workspace.display().to_string()
                }
            },
            "channels": {},
            "tools": {}
        });
        std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
            .with_context(|| format!("writing {}", config_path.display()))?;
    }

    write_if_missing(
        &workspace.join("SOUL.md"),
        "# SOUL\n\nDescribe how Zunel should sound and behave.\n",
    )?;
    write_if_missing(
        &workspace.join("USER.md"),
        "# USER\n\nCapture stable information about the user here.\n",
    )?;
    write_if_missing(
        &workspace.join("HEARTBEAT.md"),
        "# HEARTBEAT\n\n## Periodic Tasks\n\n",
    )?;
    write_if_missing(
        &workspace.join("memory").join("MEMORY.md"),
        "# MEMORY\n\nDurable project facts and decisions live here.\n",
    )?;

    println!("onboarded: {}", home.display());
    println!("config: {}", config_path.display());
    println!("workspace: {}", workspace.display());
    Ok(())
}

fn write_if_missing(path: &std::path::Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}
