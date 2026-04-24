//! Local tools for zunel: filesystem, search, shell, web, plus the
//! `Tool` trait and `ToolRegistry` everything else registers through.

pub mod error;
pub mod file_state;
pub mod fs;
pub mod path_policy;
mod registry;
pub mod schema;
pub mod ssrf;
mod tool;

pub use error::{Error, Result};
pub use registry::ToolRegistry;
pub use tool::{DynTool, Tool, ToolContext, ToolResult};
