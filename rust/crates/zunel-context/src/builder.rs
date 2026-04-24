use std::path::PathBuf;

use serde_json::{json, Value};

use zunel_skills::SkillsLoader;

use crate::error::{Error, Result};
use crate::runtime_tag::{CLOSE_TAG, OPEN_TAG};
use crate::templates::{render_identity, render_platform_policy, render_skills_section};

const SECTION_SEPARATOR: &str = "\n\n---\n\n";
const MAX_RECENT_HISTORY: usize = 50;

const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md"];

pub struct ContextBuilder {
    workspace: PathBuf,
    skills: SkillsLoader,
}

impl ContextBuilder {
    pub fn new(workspace: PathBuf, skills: SkillsLoader) -> Self {
        Self { workspace, skills }
    }

    pub fn build_system_prompt(&self, channel: Option<&str>) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();

        let runtime = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let identity = render_identity(&self.workspace.display().to_string(), &runtime, channel)?;
        parts.push(identity);

        let policy = render_platform_policy()?;
        parts.push(policy);

        for name in BOOTSTRAP_FILES {
            let path = self.workspace.join(name);
            if path.exists() {
                let body = std::fs::read_to_string(&path).map_err(Error::from)?;
                parts.push(format!("## {name}\n\n{}", body.trim_end()));
            }
        }

        let always = self.skills.get_always_skills()?;
        if !always.is_empty() {
            let blob = self.skills.load_skills_for_context(&always)?;
            if !blob.is_empty() {
                parts.push(format!("# Active Skills\n\n{blob}"));
            }
        }
        let exclude: std::collections::HashSet<String> = always.into_iter().collect();
        let summary = self.skills.build_skills_summary(Some(&exclude))?;
        if !summary.is_empty() {
            parts.push(render_skills_section(&summary)?);
        }

        Ok(parts.join(SECTION_SEPARATOR))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_messages(
        &self,
        history: &[Value],
        current_message: &str,
        media: Option<&Value>,
        channel: Option<&str>,
        chat_id: Option<&str>,
        current_role: &str,
        session_summary: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut messages: Vec<Value> = Vec::with_capacity(history.len() + 2);
        let system = self.build_system_prompt(channel)?;
        messages.push(json!({"role": "system", "content": system}));

        let start = history.len().saturating_sub(MAX_RECENT_HISTORY);
        for msg in &history[start..] {
            messages.push(msg.clone());
        }

        let runtime_block = build_runtime_block(channel, chat_id, session_summary);
        let wrapped_content = if runtime_block.is_empty() {
            current_message.to_string()
        } else {
            format!("{runtime_block}{current_message}")
        };

        let merge_with_last = messages
            .last()
            .and_then(|v| v.get("role"))
            .and_then(Value::as_str)
            == Some(current_role);

        let mut current = match (current_role, media) {
            ("user", Some(media)) => json!({
                "role": "user",
                "content": wrapped_content,
                "media": media,
            }),
            (role, _) => json!({"role": role, "content": wrapped_content}),
        };

        if merge_with_last {
            let prev = messages.pop().unwrap();
            let combined = format!(
                "{}\n\n{}",
                prev["content"].as_str().unwrap_or_default(),
                current["content"].as_str().unwrap_or_default()
            );
            current["content"] = Value::String(combined);
        }

        messages.push(current);
        Ok(messages)
    }
}

fn build_runtime_block(
    channel: Option<&str>,
    chat_id: Option<&str>,
    session_summary: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    lines.push(format!("time: {now}"));
    if let Some(c) = channel {
        lines.push(format!("channel: {c}"));
    }
    if let Some(c) = chat_id {
        lines.push(format!("chat_id: {c}"));
    }
    if let Some(s) = session_summary {
        if !s.is_empty() {
            lines.push(format!("summary: {s}"));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("{OPEN_TAG}\n{}\n{CLOSE_TAG}\n", lines.join("\n"))
}
