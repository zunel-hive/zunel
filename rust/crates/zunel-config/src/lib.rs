//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod paths;
mod schema;

pub use error::{Error, Result};
pub use paths::{default_config_path, zunel_home};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
