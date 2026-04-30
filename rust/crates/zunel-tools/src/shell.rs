use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use regex::RegexSet;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::tool::{Tool, ToolContext, ToolResult};

const MAX_TIMEOUT_S: u64 = 600;
const DEFAULT_TIMEOUT_S: u64 = 60;
const MAX_OUTPUT: usize = 10_000;

const DEFAULT_DENY: &[&str] = &[
    r"\brm\s+-[rf]{1,2}\b",
    r"\bdel\s+/[fq]\b",
    r"\brmdir\s+/s\b",
    r"(?:^|[;&|]\s*)format\b",
    r"\b(mkfs|diskpart)\b",
    r"\bdd\s+if=",
    r">\s*/dev/sd",
    r"\b(shutdown|reboot|poweroff)\b",
    r":\(\)\s*\{.*\};\s*:",
    r">>?\s*\S*(?:history\.jsonl|\.dream_cursor)",
    r"\btee\b[^|;&<>]*(?:history\.jsonl|\.dream_cursor)",
    r"\b(?:cp|mv)\b(?:\s+[^\s|;&<>]+)+\s+\S*(?:history\.jsonl|\.dream_cursor)",
    r"\bdd\b[^|;&<>]*\bof=\S*(?:history\.jsonl|\.dream_cursor)",
    r"\bsed\s+-i[^|;&<>]*(?:history\.jsonl|\.dream_cursor)",
];

pub struct ExecTool {
    deny: RegexSet,
    bwrap_present: bool,
}

impl ExecTool {
    pub fn new_default() -> Self {
        let deny = RegexSet::new(DEFAULT_DENY).expect("default deny regex compiles");
        let bwrap_present = which::which("bwrap").is_ok();
        Self {
            deny,
            bwrap_present,
        }
    }
}

impl Default for ExecTool {
    fn default() -> Self {
        Self::new_default()
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &'static str {
        "exec"
    }
    fn description(&self) -> &'static str {
        "Execute a shell command. Use -y/--yes flags to avoid interactive prompts. \
         Output capped at 10 000 chars; default timeout 60s, max 600s."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "working_dir": {"type": "string"},
                "timeout": {"type": "integer", "description": "seconds, max 600"},
            },
            "required": ["command"],
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(cmd) = args.get("command").and_then(Value::as_str) else {
            return ToolResult::err("exec: missing command".to_string());
        };
        if self.deny.is_match(cmd) {
            return ToolResult::err(format!("exec: command denied by safety policy: {cmd}"));
        }
        let timeout_s = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_S)
            .min(MAX_TIMEOUT_S);
        let cwd = args
            .get("working_dir")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.workspace.clone());

        let (program, full_args) = if self.bwrap_present {
            let args_vec: Vec<String> = vec![
                "--dev".into(),
                "/dev".into(),
                "--proc".into(),
                "/proc".into(),
                "--ro-bind".into(),
                "/usr".into(),
                "/usr".into(),
                "--ro-bind".into(),
                "/bin".into(),
                "/bin".into(),
                "--ro-bind".into(),
                "/lib".into(),
                "/lib".into(),
                "--bind".into(),
                cwd.display().to_string(),
                cwd.display().to_string(),
                "--chdir".into(),
                cwd.display().to_string(),
                "/bin/sh".into(),
                "-c".into(),
                cmd.into(),
            ];
            ("bwrap".to_string(), args_vec)
        } else {
            ("/bin/sh".to_string(), vec!["-c".into(), cmd.into()])
        };

        let mut command = Command::new(&program);
        command.args(&full_args).current_dir(&cwd);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = match command.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("exec: spawn failed: {e}")),
        };
        let output_fut = child.wait_with_output();
        let output = match timeout(Duration::from_secs(timeout_s), output_fut).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return ToolResult::err(format!("exec: runtime error: {e}")),
            Err(_) => return ToolResult::err(format!("exec: timed out after {timeout_s}s")),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut combined = if !stderr.is_empty() {
            format!("{stdout}\n--- stderr ---\n{stderr}")
        } else {
            stdout.to_string()
        };

        if combined.len() > MAX_OUTPUT {
            combined.truncate(MAX_OUTPUT);
            combined.push_str("\n[output truncated at 10 000 chars]\n");
        }

        if !output.status.success() {
            combined.push_str(&format!(
                "\nexit status: {}\n",
                output.status.code().unwrap_or(-1)
            ));
        }

        ToolResult::ok(combined)
    }
}
