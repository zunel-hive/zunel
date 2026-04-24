//! Public Rust library facade for zunel.
//!
//! ```no_run
//! use zunel::Zunel;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let bot = Zunel::from_config(None).await?;
//! let result = bot.run("Summarize this repo.").await?;
//! println!("{}", result.content);
//! # Ok(()) }
//! ```

use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

pub use zunel_config::{Config, Error as ConfigError};
pub use zunel_core::{
    AgentLoop, ChatRole, CommandContext, CommandOutcome, CommandRouter, Error as CoreError,
    RunResult, Session, SessionManager,
};
pub use zunel_providers::{Error as ProviderError, LLMProvider, StreamEvent};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Core(#[from] CoreError),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Zunel {
    inner: AgentLoop,
}

impl Zunel {
    /// Build a `Zunel` instance from a config file. If `path` is `None`, uses
    /// `<zunel_home>/config.json`.
    pub async fn from_config(path: Option<&Path>) -> Result<Self> {
        let cfg = zunel_config::load_config(path)?;
        let workspace = zunel_config::workspace_path(&cfg.agents.defaults)?;
        zunel_util::ensure_dir(&workspace).map_err(|source| CoreError::Session {
            path: workspace.clone(),
            source: Box::new(source),
        })?;
        let provider: Arc<dyn LLMProvider> = zunel_providers::build_provider(&cfg)?;
        let sessions = SessionManager::new(&workspace);
        let inner = AgentLoop::with_sessions(provider, cfg.agents.defaults, sessions);
        Ok(Self { inner })
    }

    /// One-shot: run a single prompt with no session persistence.
    /// Kept for slice-1 compatibility; prefer `run_streamed` for new code.
    pub async fn run(&self, message: &str) -> Result<RunResult> {
        Ok(self.inner.process_direct(message).await?)
    }

    /// Streaming turn with session persistence. Deltas arrive on `sink`;
    /// the final `RunResult` returns when the turn ends. Drop or close
    /// the receiver on `sink` to propagate-cancel the render consumer;
    /// the provider stream always runs to completion so the session
    /// file remains consistent.
    pub async fn run_streamed(
        &self,
        session_key: &str,
        message: &str,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<RunResult> {
        Ok(self
            .inner
            .process_streamed(session_key, message, sink)
            .await?)
    }
}
