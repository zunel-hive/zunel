use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{Error, Result};

const CODEX_LOGIN_HINT: &str =
    "Sign in with `codex login` using file-backed credentials, then retry.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: String,
}

#[async_trait]
pub trait CodexAuthProvider: Send + Sync {
    async fn load(&self) -> Result<CodexAuth>;
}

#[derive(Debug, Clone)]
pub struct FileCodexAuthProvider {
    codex_home: PathBuf,
}

impl FileCodexAuthProvider {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn from_env() -> Result<Self> {
        if let Ok(home) = std::env::var("CODEX_HOME") {
            return Ok(Self::new(PathBuf::from(home)));
        }
        let home = std::env::var("HOME").map_err(|_| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: HOME is not set. {CODEX_LOGIN_HINT}"
            ))
        })?;
        Ok(Self::new(PathBuf::from(home).join(".codex")))
    }

    fn auth_path(&self) -> PathBuf {
        self.codex_home.join("auth.json")
    }
}

#[async_trait]
impl CodexAuthProvider for FileCodexAuthProvider {
    async fn load(&self) -> Result<CodexAuth> {
        let path = self.auth_path();
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: failed to read {}: {e}. {CODEX_LOGIN_HINT}",
                path.display()
            ))
        })?;
        let value: Value = serde_json::from_str(&raw).map_err(|e| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: failed to parse {}: {e}. {CODEX_LOGIN_HINT}",
                path.display()
            ))
        })?;
        let access_token = value
            .pointer("/tokens/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                Error::Auth(format!(
                    "Codex OAuth credentials unavailable: auth.json does not contain an access token. {CODEX_LOGIN_HINT}"
                ))
            })?
            .to_string();
        let account_id = find_account_id(&value).ok_or_else(|| {
            Error::Auth(format!(
                "Codex OAuth credentials unavailable: auth.json does not contain a ChatGPT account id. {CODEX_LOGIN_HINT}"
            ))
        })?;

        Ok(CodexAuth {
            access_token,
            account_id,
        })
    }
}

fn find_account_id(value: &Value) -> Option<String> {
    [
        "/account_id",
        "/chatgpt_account_id",
        "/account/id",
        "/profile/account_id",
        "/tokens/account_id",
    ]
    .iter()
    .find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    })
}
