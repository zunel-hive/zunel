//! Agent loop, runner, context, memory.

mod agent_loop;
pub mod approval;
pub mod command;
mod error;
mod session;

pub use agent_loop::{AgentLoop, RunResult};
pub use approval::{
    summarize_tool_call, tool_requires_approval, AllowAllApprovalHandler, ApprovalDecision,
    ApprovalHandler, ApprovalRequest, ApprovalScope, CachedApprovalHandler,
};
pub use command::{CommandContext, CommandOutcome, CommandRouter};
pub use error::{Error, Result};
pub use session::{ChatRole, Session, SessionManager};
