//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod schema;

pub use error::{Error, Result};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
