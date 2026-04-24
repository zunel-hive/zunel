use std::path::PathBuf;

use crate::error::{Error, Result};

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
