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

pub use zunel_config::{Config, Error as ConfigError};
pub use zunel_core::{AgentLoop, Error as CoreError, RunResult};
pub use zunel_providers::{Error as ProviderError, LLMProvider};

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
    loop_inner: AgentLoop,
}

impl Zunel {
    /// Build a `Zunel` instance from a config file. If `path` is `None`, uses
    /// `<zunel_home>/config.json`.
    pub async fn from_config(path: Option<&Path>) -> Result<Self> {
        let cfg = zunel_config::load_config(path)?;
        let provider: Arc<dyn LLMProvider> = zunel_providers::build_provider(&cfg)?;
        let loop_inner = AgentLoop::new(provider, cfg.agents.defaults);
        Ok(Self { loop_inner })
    }

    /// Run a single prompt against the configured provider.
    pub async fn run(&self, message: &str) -> Result<RunResult> {
        Ok(self.loop_inner.process_direct(message).await?)
    }
}
