use chrono::{DateTime, Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Canonical chat role string mapping used across sessions.
/// Kept simple on purpose — the same four roles Python uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

/// Python-compatible naive local timestamp, e.g. ``2026-04-24T10:58:27.123456``.
///
/// Matches ``datetime.now().isoformat()`` exactly: local wall-clock time,
/// microsecond precision, no timezone suffix.
pub(crate) fn naive_local_iso_now() -> String {
    naive_local_iso(Local::now())
}

pub(crate) fn naive_local_iso(ts: DateTime<Local>) -> String {
    let naive: NaiveDateTime = ts.naive_local();
    naive.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// A single conversation session, byte-compatible with Python's
/// `zunel/session/manager.py::Session`.
#[derive(Debug, Clone)]
pub struct Session {
    key: String,
    messages: Vec<Value>,
    created_at: String,
    updated_at: String,
    metadata: Value,
    last_consolidated: usize,
}

impl Session {
    pub fn new(key: impl Into<String>) -> Self {
        let now = naive_local_iso_now();
        Self {
            key: key.into(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            metadata: Value::Object(Default::default()),
            last_consolidated: 0,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    pub fn last_consolidated(&self) -> usize {
        self.last_consolidated
    }

    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }

    pub fn metadata(&self) -> &Value {
        &self.metadata
    }

    pub fn add_message(&mut self, role: ChatRole, content: impl Into<String>) {
        let msg = json!({
            "role": role.as_str(),
            "content": content.into(),
            "timestamp": naive_local_iso_now(),
        });
        self.messages.push(msg);
        self.updated_at = naive_local_iso_now();
    }

    /// Return up to ``max_messages`` of the most recent unconsolidated
    /// messages, stripped of the ``timestamp`` field (LLMs don't need it).
    pub fn get_history(&self, max_messages: usize) -> Vec<Value> {
        let unconsolidated = &self.messages[self.last_consolidated..];
        let start = unconsolidated.len().saturating_sub(max_messages);
        unconsolidated[start..]
            .iter()
            .map(|m| {
                let mut clone = m.clone();
                if let Some(obj) = clone.as_object_mut() {
                    obj.remove("timestamp");
                }
                clone
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.last_consolidated = 0;
        self.updated_at = naive_local_iso_now();
    }

    // Internal constructor used by `SessionManager::load`.
    pub(crate) fn from_parts(
        key: String,
        messages: Vec<Value>,
        created_at: String,
        updated_at: String,
        metadata: Value,
        last_consolidated: usize,
    ) -> Self {
        Self {
            key,
            messages,
            created_at,
            updated_at,
            metadata,
            last_consolidated,
        }
    }

    pub(crate) fn metadata_line(&self) -> Value {
        // Must preserve field order used by Python:
        // _type, key, created_at, updated_at, metadata, last_consolidated.
        json!({
            "_type": "metadata",
            "key": self.key,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "metadata": self.metadata,
            "last_consolidated": self.last_consolidated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_format_matches_python_shape() {
        let ts: DateTime<Local> = chrono::TimeZone::with_ymd_and_hms(
            &Local, 2026, 4, 24, 10, 58, 27,
        )
        .unwrap();
        let iso = naive_local_iso(ts);
        // Microsecond precision, no tz suffix.
        assert_eq!(iso, "2026-04-24T10:58:27.000000");
    }
}
