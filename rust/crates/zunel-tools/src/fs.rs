use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::path_policy::PathPolicy;
use crate::tool::{Tool, ToolContext, ToolResult};

fn resolve_path(policy: &PathPolicy, ctx: &ToolContext, raw: &str) -> Result<PathBuf, String> {
    let as_path = Path::new(raw);
    let abs: PathBuf = if as_path.is_absolute() {
        as_path.to_path_buf()
    } else {
        ctx.workspace.join(as_path)
    };
    policy.check(&abs).map_err(|e| e.to_string())
}

pub struct ReadFileTool {
    policy: PathPolicy,
}

impl ReadFileTool {
    pub fn new(policy: PathPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a text file from the workspace. Returns contents with optional offset/limit line pagination."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Workspace-relative or absolute path."},
                "offset": {"type": "integer", "description": "Zero-based first line to include.", "default": 0},
                "limit": {"type": "integer", "description": "Max lines to include.", "default": 2000},
            },
            "required": ["path"],
        })
    }
    fn concurrency_safe(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = match args.get("path").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolResult::err("read_file: missing path".to_string()),
        };
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(2000) as usize;
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("read_file: {msg}")),
        };
        let body = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) => return ToolResult::err(format!("read_file: {e} ({path:?})")),
        };
        let lines: Vec<&str> = body.lines().skip(offset).take(limit).collect();
        let mut out = lines.join("\n");
        if !out.ends_with('\n') && !body.is_empty() {
            out.push('\n');
        }
        ToolResult::ok(out)
    }
}

pub struct WriteFileTool {
    policy: PathPolicy,
}

impl WriteFileTool {
    pub fn new(policy: PathPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a workspace file with the given UTF-8 contents."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"},
            },
            "required": ["path", "content"],
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = match args.get("path").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolResult::err("write_file: missing path".to_string()),
        };
        let content = args.get("content").and_then(Value::as_str).unwrap_or("");
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("write_file: {msg}")),
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(format!("write_file: mkdir {parent:?}: {e}"));
            }
        }
        match tokio::fs::write(&path, content).await {
            Ok(()) => ToolResult::ok(format!(
                "wrote {} bytes to {}",
                content.len(),
                path.display()
            )),
            Err(e) => ToolResult::err(format!("write_file: {e}")),
        }
    }
}

pub struct ListDirTool {
    policy: PathPolicy,
}

impl ListDirTool {
    pub fn new(policy: PathPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List files and sub-directories in a workspace directory."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Dir to list (default '.')"},
            },
            "required": [],
        })
    }
    fn concurrency_safe(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("list_dir: {msg}")),
        };
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(e) => e,
            Err(err) => return ToolResult::err(format!("list_dir: {err} ({path:?})")),
        };
        let mut names: Vec<String> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let is_dir = entry.file_type().await.map(|f| f.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            if is_dir {
                names.push(format!("{name}/"));
            } else {
                names.push(name);
            }
        }
        names.sort();
        ToolResult::ok(names.join("\n"))
    }
}
