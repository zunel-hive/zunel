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
    /// One-shot message to send. Without this, drops into an interactive REPL.
    #[arg(short = 'm', long = "message")]
    pub message: Option<String>,

    /// Session ID (channel:chat_id). Defaults to `cli:direct`.
    #[arg(short = 's', long = "session", default_value = "cli:direct")]
    pub session: String,
}
