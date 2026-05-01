use std::collections::BTreeMap;
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
    /// Pre-resolved env vars layered on top of the parent process's
    /// environment for every spawned command. Built from
    /// `cfg.tools.exec.env` with `${VAR}` / `${VAR:-default}`
    /// substitution already applied at registry-build time so we
    /// don't re-walk the user's config on every tool call.
    extra_env: Vec<(String, String)>,
}

impl ExecTool {
    pub fn new_default() -> Self {
        Self::with_env(BTreeMap::new())
    }

    /// Construct an `ExecTool` that injects the given env map into
    /// every spawned command, on top of the parent process's
    /// environment. Values may reference other env vars via `${VAR}`
    /// or `${VAR:-default}`; substitution is performed against
    /// `std::env::var` at construction time.
    pub fn with_env(env: BTreeMap<String, String>) -> Self {
        Self::with_env_using(env, |name| std::env::var(name).ok())
    }

    /// Same as [`Self::with_env`] but with an injectable env lookup
    /// for unit tests.
    pub fn with_env_using(
        env: BTreeMap<String, String>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let deny = RegexSet::new(DEFAULT_DENY).expect("default deny regex compiles");
        let bwrap_present = which::which("bwrap").is_ok();
        let extra_env = compose_exec_env(env, &lookup);
        Self {
            deny,
            bwrap_present,
            extra_env,
        }
    }
}

impl Default for ExecTool {
    fn default() -> Self {
        Self::new_default()
    }
}

/// Resolve a `tools.exec.env` map into a flat `(KEY, VALUE)` list ready to
/// be fed into `Command::envs(...)`. `${VAR}` and `${VAR:-default}` tokens
/// inside each *value* are expanded against `lookup`; missing variables
/// without a `:-default` fall back to the empty string (intentionally
/// permissive so a misconfigured `${PATH}` doesn't make the command
/// completely unspawnable). `$$` is a literal `$`. Bare `$` not followed
/// by `{` or `$` is left alone so PHC-style argon2 / k8s template values
/// survive.
fn compose_exec_env(
    env: BTreeMap<String, String>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    env.into_iter()
        .map(|(key, raw)| (key, expand_env_placeholders(&raw, lookup)))
        .collect()
}

fn expand_env_placeholders(raw: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            let next = bytes[i..]
                .iter()
                .position(|&b| b == b'$')
                .map(|p| i + p)
                .unwrap_or(bytes.len());
            out.push_str(&raw[i..next]);
            i = next;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'$') => {
                out.push('$');
                i += 2;
            }
            Some(b'{') => {
                let Some(close_rel) = bytes[i + 2..].iter().position(|&b| b == b'}') else {
                    out.push('$');
                    i += 1;
                    continue;
                };
                let close = i + 2 + close_rel;
                let inside = &raw[i + 2..close];
                let (var_name, default) = match inside.split_once(":-") {
                    Some((name, default)) => (name.trim(), Some(default)),
                    None => (inside.trim(), None),
                };
                if !valid_env_var_name(var_name) {
                    out.push('$');
                    i += 1;
                    continue;
                }
                match lookup(var_name) {
                    Some(value) if !value.is_empty() => out.push_str(&value),
                    _ => {
                        if let Some(default) = default {
                            out.push_str(default);
                        }
                    }
                }
                i = close + 1;
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

fn valid_env_var_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
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
        if !self.extra_env.is_empty() {
            command.envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContext;

    fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn compose_exec_env_passes_through_static_values() {
        let mut env = BTreeMap::new();
        env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        let resolved = compose_exec_env(env, &lookup_from(&[]));
        assert_eq!(
            resolved,
            vec![("LANG".to_string(), "en_US.UTF-8".to_string())]
        );
    }

    #[test]
    fn compose_exec_env_extends_path_via_substitution() {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), "$HOME/.cargo/bin:${PATH}".to_string());
        let resolved =
            compose_exec_env(env, &lookup_from(&[("PATH", "/opt/homebrew/bin:/usr/bin")]));
        // Note: bare `$HOME` (no `{}`) is left alone — `/bin/sh -c` will
        // expand it in the shell. The `${PATH}` token IS expanded
        // because we do that pre-spawn.
        assert_eq!(
            resolved,
            vec![(
                "PATH".to_string(),
                "$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin".to_string()
            )]
        );
    }

    #[test]
    fn compose_exec_env_uses_default_when_var_missing() {
        let mut env = BTreeMap::new();
        env.insert(
            "TZ".to_string(),
            "${USER_TZ:-America/Los_Angeles}".to_string(),
        );
        let resolved = compose_exec_env(env, &lookup_from(&[]));
        assert_eq!(
            resolved,
            vec![("TZ".to_string(), "America/Los_Angeles".to_string())]
        );
    }

    #[test]
    fn compose_exec_env_missing_var_no_default_expands_to_empty() {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), "/extra:${MISSING}".to_string());
        let resolved = compose_exec_env(env, &lookup_from(&[]));
        // Permissive behavior: empty expansion rather than dropping the
        // var. Documented in ExecToolsConfig::env's doc comment.
        assert_eq!(resolved, vec![("PATH".to_string(), "/extra:".to_string())]);
    }

    #[test]
    fn compose_exec_env_double_dollar_is_literal() {
        let mut env = BTreeMap::new();
        env.insert("PROMPT".to_string(), "$$ marker $${X}".to_string());
        let resolved = compose_exec_env(env, &lookup_from(&[("X", "should-not-appear")]));
        assert_eq!(
            resolved,
            vec![("PROMPT".to_string(), "$ marker ${X}".to_string())]
        );
    }

    #[test]
    fn compose_exec_env_passes_bare_dollar_through() {
        let mut env = BTreeMap::new();
        env.insert(
            "ARGON".to_string(),
            "$argon2id$v=19$m=65536$abc".to_string(),
        );
        let resolved = compose_exec_env(env, &lookup_from(&[]));
        assert_eq!(
            resolved,
            vec![(
                "ARGON".to_string(),
                "$argon2id$v=19$m=65536$abc".to_string()
            )]
        );
    }

    #[test]
    fn compose_exec_env_unterminated_brace_is_left_intact() {
        let mut env = BTreeMap::new();
        env.insert("BROKEN".to_string(), "value=${UNCLOSED".to_string());
        let resolved = compose_exec_env(env, &lookup_from(&[("UNCLOSED", "x")]));
        // Best-effort: emit a literal `$` and continue, instead of
        // dropping the entry entirely.
        assert_eq!(
            resolved,
            vec![("BROKEN".to_string(), "value=${UNCLOSED".to_string())]
        );
    }

    #[test]
    fn valid_env_var_name_accepts_posix_identifiers() {
        assert!(valid_env_var_name("PATH"));
        assert!(valid_env_var_name("_X"));
        assert!(valid_env_var_name("FOO_BAR_2"));
        assert!(!valid_env_var_name(""));
        assert!(!valid_env_var_name("9NOPE"));
        assert!(!valid_env_var_name("FOO-BAR"));
    }

    #[tokio::test]
    async fn exec_tool_exposes_extra_env_to_child_shell() {
        // Skip on hosts without /bin/sh (Windows CI). Linux + macOS are
        // both fine; bwrap-equipped Linux environments would route
        // through bwrap which carries the parent env through too, but
        // the bwrap mount layout in tests doesn't have $HOME mapped, so
        // we explicitly construct an ExecTool that bypasses bwrap.
        if !std::path::Path::new("/bin/sh").exists() {
            return;
        }

        let mut env = BTreeMap::new();
        env.insert(
            "ZUNEL_TEST_PATH".to_string(),
            "/zunel-test/bin:/usr/bin".to_string(),
        );
        env.insert("ZUNEL_TEST_GREETING".to_string(), "hello".to_string());

        let tool = ExecTool {
            deny: RegexSet::new(DEFAULT_DENY).unwrap(),
            bwrap_present: false,
            extra_env: compose_exec_env(env, &|_| None),
        };

        let ctx = ToolContext::for_test();
        let res = tool
            .execute(
                json!({
                    "command": "printf '%s|%s' \"$ZUNEL_TEST_GREETING\" \"$ZUNEL_TEST_PATH\""
                }),
                &ctx,
            )
            .await;

        assert!(!res.is_error, "exec failed: {}", res.content);
        assert!(
            res.content.contains("hello|/zunel-test/bin:/usr/bin"),
            "child shell did not see configured env vars; got: {}",
            res.content
        );
    }
}
