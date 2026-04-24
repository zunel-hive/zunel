use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "zunel",
    version,
    about = "zunel — a lean personal AI assistant"
)]
pub struct Cli {
    /// Override the config file path (default: ~/.zunel/config.json).
    #[arg(long, global = true, env = "ZUNEL_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the agent against a one-shot prompt.
    Agent(AgentArgs),
}

#[derive(Debug, Parser)]
pub struct AgentArgs {
    /// One-shot message to send.
    #[arg(short = 'm', long = "message")]
    pub message: String,
}
