use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::schema::AgentDefaults;

/// Resolve the zunel home directory.
///
/// Precedence:
/// 1. `ZUNEL_HOME` env var (used for tests and custom installs).
/// 2. `$HOME/.zunel` on Unix, platform-appropriate home on other OSes.
pub fn zunel_home() -> Result<PathBuf> {
    if let Some(val) = std::env::var_os("ZUNEL_HOME") {
        return Ok(PathBuf::from(val));
    }
    let home = dirs::home_dir().ok_or_else(|| Error::Io {
        path: PathBuf::from("~"),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve home directory",
        ),
    })?;
    Ok(home.join(".zunel"))
}

/// Default config file path: `<zunel_home>/config.json`.
pub fn default_config_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("config.json"))
}

/// Default workspace location: `<zunel_home>/workspace/`.
pub fn default_workspace_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("workspace"))
}

/// Resolve the workspace path from config.
///
/// Precedence:
/// 1. ``agents.defaults.workspace`` if set (with ``~`` expansion).
/// 2. ``<zunel_home>/workspace/``.
pub fn workspace_path(defaults: &AgentDefaults) -> Result<PathBuf> {
    match defaults.workspace.as_deref() {
        Some(raw) => Ok(expand_tilde(raw)),
        None => default_workspace_path(),
    }
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(raw)
}

/// Sessions directory inside a workspace: `<workspace>/sessions/`.
/// Matches Python's `SessionManager.__init__` layout byte-for-byte.
pub fn sessions_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.join("sessions")
}

/// Persistent REPL history file: `<zunel_home>/cli_history.txt`.
/// Matches Python's `get_cli_history_path`.
pub fn cli_history_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("cli_history.txt"))
}
