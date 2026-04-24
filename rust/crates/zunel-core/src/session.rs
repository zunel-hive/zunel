use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime};
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

    /// Test-only constructor for deterministic fixtures.
    #[doc(hidden)]
    pub fn for_test(
        key: String,
        messages: Vec<Value>,
        created_at: String,
        updated_at: String,
    ) -> Self {
        Self {
            key,
            messages,
            created_at,
            updated_at,
            metadata: Value::Object(Default::default()),
            last_consolidated: 0,
        }
    }
}

/// Owns the `<workspace>/sessions/` directory and performs atomic load + save.
///
/// Byte-compatible with Python's `zunel/session/manager.py::SessionManager`.
#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions_dir: PathBuf,
}

impl SessionManager {
    pub fn new(workspace: &Path) -> Self {
        let sessions_dir = zunel_config::sessions_dir(workspace);
        if let Err(e) = zunel_util::ensure_dir(&sessions_dir) {
            tracing::warn!(path = %sessions_dir.display(), error = %e, "failed to create sessions dir");
        }
        Self { sessions_dir }
    }

    /// Python-compat: replace path-unsafe characters with `_` and trim
    /// surrounding whitespace. Mirrors `zunel/utils/helpers.py::safe_filename`
    /// applied to `key.replace(":", "_")`. The unsafe set matches Python's
    /// `_UNSAFE_CHARS = re.compile(r'[<>:"/\\|?*]')`, so ordinary letters,
    /// digits, spaces, periods, and non-ASCII codepoints pass through
    /// unchanged, keeping Rust-generated session filenames byte-compatible
    /// with Python-generated ones.
    fn safe_key(key: &str) -> String {
        const UNSAFE: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
        key.chars()
            .map(|c| if UNSAFE.contains(&c) { '_' } else { c })
            .collect::<String>()
            .trim()
            .to_string()
    }

    fn session_path(&self, key: &str) -> PathBuf {
        self.sessions_dir
            .join(format!("{}.jsonl", Self::safe_key(key)))
    }

    pub fn load(&self, key: &str) -> crate::Result<Option<Session>> {
        let path = self.session_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let file = File::open(&path).map_err(|source| crate::Error::Session {
            path: path.clone(),
            source: source.into(),
        })?;
        let reader = BufReader::new(file);

        let mut messages = Vec::new();
        let mut key_from_file: Option<String> = None;
        let mut created_at: Option<String> = None;
        let mut updated_at: Option<String> = None;
        let mut metadata = Value::Object(Default::default());
        let mut last_consolidated = 0usize;

        for line in reader.lines() {
            let line = line.map_err(|source| crate::Error::Session {
                path: path.clone(),
                source: source.into(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|source| crate::Error::Session {
                    path: path.clone(),
                    source: source.into(),
                })?;
            if value.get("_type").and_then(Value::as_str) == Some("metadata") {
                key_from_file = value.get("key").and_then(Value::as_str).map(str::to_string);
                created_at = value
                    .get("created_at")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                updated_at = value
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(m) = value.get("metadata") {
                    metadata = m.clone();
                }
                last_consolidated = value
                    .get("last_consolidated")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
            } else {
                messages.push(value);
            }
        }

        let now = naive_local_iso_now();
        Ok(Some(Session::from_parts(
            key_from_file.unwrap_or_else(|| key.to_string()),
            messages,
            created_at.unwrap_or_else(|| now.clone()),
            updated_at.unwrap_or(now),
            metadata,
            last_consolidated,
        )))
    }

    pub fn save(&self, session: &Session) -> crate::Result<()> {
        let path = self.session_path(&session.key);
        let tmp_path = path.with_extension("jsonl.tmp");

        // Python writes with `json.dumps(..., ensure_ascii=False)` whose
        // default separators are `(", ", ": ")`. serde_json's compact
        // writer uses `(",", ":")`. We hand-serialize line by line with
        // Python's spacing via `PythonWriter`.
        let mut file = File::create(&tmp_path).map_err(|source| crate::Error::Session {
            path: tmp_path.clone(),
            source: source.into(),
        })?;
        write_python_json(&mut file, &session.metadata_line())?;
        file.write_all(b"\n")
            .map_err(|source| crate::Error::Session {
                path: tmp_path.clone(),
                source: source.into(),
            })?;
        for msg in &session.messages {
            write_python_json(&mut file, msg)?;
            file.write_all(b"\n")
                .map_err(|source| crate::Error::Session {
                    path: tmp_path.clone(),
                    source: source.into(),
                })?;
        }
        drop(file);

        fs::rename(&tmp_path, &path).map_err(|source| crate::Error::Session {
            path: path.clone(),
            source: source.into(),
        })?;
        Ok(())
    }
}

/// Emit JSON with Python's default `json.dumps(ensure_ascii=False)`
/// separators: `", "` between items, `": "` between keys and values.
/// This is the minimum we need for byte-compat with Python zunel.
fn write_python_json(writer: &mut impl Write, value: &Value) -> crate::Result<()> {
    let formatter = PythonCompactFormatter;
    let mut ser = serde_json::Serializer::with_formatter(writer, formatter);
    serde::Serialize::serialize(value, &mut ser).map_err(|source| crate::Error::Session {
        path: PathBuf::new(),
        source: source.into(),
    })?;
    Ok(())
}

struct PythonCompactFormatter;

impl serde_json::ser::Formatter for PythonCompactFormatter {
    fn begin_array_value<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(b": ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn iso_format_matches_python_shape() {
        // Non-zero microseconds prove the `%.6f` format actually carries
        // sub-second precision through to the output, not just that the
        // happy path prints six zeros.
        let naive = NaiveDate::from_ymd_opt(2026, 4, 24)
            .unwrap()
            .and_hms_micro_opt(10, 58, 27, 123_456)
            .unwrap();
        let ts: DateTime<Local> = naive.and_local_timezone(Local).unwrap();
        let iso = naive_local_iso(ts);
        // Microsecond precision, no tz suffix — matches Python's
        // `datetime.now().isoformat()` for non-zero-microsecond values.
        assert_eq!(iso, "2026-04-24T10:58:27.123456");
    }

    #[test]
    fn safe_key_matches_python_safe_filename() {
        // Colons become underscores (SessionManager's standard namespacing).
        assert_eq!(SessionManager::safe_key("cli:direct"), "cli_direct");
        assert_eq!(SessionManager::safe_key("agent:foo:bar"), "agent_foo_bar");
        // Windows-unsafe chars are stripped, everything else passes through,
        // including periods, spaces, and non-ASCII codepoints.
        assert_eq!(
            SessionManager::safe_key("hello world.txt"),
            "hello world.txt"
        );
        assert_eq!(SessionManager::safe_key("dir/name"), "dir_name");
        assert_eq!(SessionManager::safe_key("a<b>c|d?e*f"), "a_b_c_d_e_f");
        assert_eq!(SessionManager::safe_key("émoji🎉"), "émoji🎉");
        // Surrounding whitespace is trimmed, like Python's .strip().
        assert_eq!(SessionManager::safe_key("  padded  "), "padded");
    }
}
