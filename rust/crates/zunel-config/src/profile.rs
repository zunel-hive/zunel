use std::path::PathBuf;

use crate::error::{Error, Result};

pub const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_HOME_NAME: &str = ".zunel";
const PROFILES_DIR_NAME: &str = "profiles";
const ACTIVE_PROFILE_FILE: &str = "active_profile";

pub fn default_zunel_root() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| Error::Io {
        path: PathBuf::from("~"),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve home directory",
        ),
    })?;
    Ok(home.join(DEFAULT_HOME_NAME))
}

pub fn resolve_profile_home(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name == DEFAULT_PROFILE_NAME {
        return default_zunel_root();
    }
    validate_profile_name(name)?;
    Ok(default_zunel_root()?.join(PROFILES_DIR_NAME).join(name))
}

pub fn active_profile_path() -> Result<PathBuf> {
    Ok(default_zunel_root()?.join(ACTIVE_PROFILE_FILE))
}

pub fn read_sticky_profile() -> Option<String> {
    let path = active_profile_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let name = raw.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub fn set_sticky_profile(name: &str) -> Result<()> {
    let path = active_profile_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    if name.is_empty() || name == DEFAULT_PROFILE_NAME {
        if let Err(source) = std::fs::remove_file(&path) {
            if source.kind() != std::io::ErrorKind::NotFound {
                return Err(Error::Io { path, source });
            }
        }
        return Ok(());
    }
    validate_profile_name(name)?;
    std::fs::write(&path, format!("{name}\n")).map_err(|source| Error::Io { path, source })
}

pub fn active_profile_name() -> String {
    if let Some(home) = std::env::var_os("ZUNEL_HOME") {
        let path = PathBuf::from(home);
        if let Ok(default) = default_zunel_root() {
            if path == default {
                return DEFAULT_PROFILE_NAME.to_string();
            }
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            return name.to_string();
        }
    }
    read_sticky_profile().unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string())
}

pub fn active_profile_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("ZUNEL_HOME") {
        return Ok(PathBuf::from(home));
    }
    match read_sticky_profile() {
        Some(profile) => resolve_profile_home(&profile),
        None => default_zunel_root(),
    }
}

pub fn list_profiles() -> Result<Vec<String>> {
    let root = default_zunel_root()?;
    let mut profiles = Vec::new();
    if root.exists() {
        profiles.push(DEFAULT_PROFILE_NAME.to_string());
    }
    let profiles_dir = root.join(PROFILES_DIR_NAME);
    if !profiles_dir.exists() {
        return Ok(profiles);
    }
    for entry in std::fs::read_dir(&profiles_dir).map_err(|source| Error::Io {
        path: profiles_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            path: profiles_dir.clone(),
            source,
        })?;
        if !entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_profile_name(&name).is_ok() {
            profiles.push(name);
        }
    }
    profiles.sort();
    profiles.dedup();
    Ok(profiles)
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.chars().any(char::is_whitespace)
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(Error::InvalidProfileName(name.to_string()));
    }
    Ok(())
}
