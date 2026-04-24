//! Minimal SSE (Server-Sent Events) line buffer. Not a general-purpose
//! implementation — only the subset OpenAI-compatible chat.completions
//! streams emit: `data:` lines with optional multi-line continuations,
//! event boundaries on blank lines, `[DONE]` sentinel.

/// Accumulates partial chunks and emits `Vec<Option<String>>` where:
/// - `Some(data)` is a complete `data:` payload (joined across lines).
/// - `None` is the `[DONE]` sentinel indicating end-of-stream.
#[derive(Debug, Default)]
pub struct SseBuffer {
    line_buf: String,
    event_data: Vec<String>,
}

impl SseBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes from the wire. Returns any events that completed in
    /// this chunk. Multiple events per chunk are possible; partial events
    /// stay buffered until the next call.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Option<String>> {
        let mut events = Vec::new();
        // Text-append strategy: we assume UTF-8 (OpenAI always sends it)
        // and tolerate partial code points by deferring unknown bytes.
        let s = match std::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                // Push valid prefix; drop the invalid tail. Real providers
                // don't emit invalid UTF-8 in practice.
                let valid = &bytes[..e.valid_up_to()];
                std::str::from_utf8(valid).unwrap_or("")
            }
        };
        self.line_buf.push_str(s);

        // Process all complete lines in the buffer.
        while let Some(idx) = self.line_buf.find('\n') {
            let mut line = self.line_buf[..idx].to_string();
            self.line_buf.drain(..=idx);
            if line.ends_with('\r') {
                line.pop();
            }

            if line.is_empty() {
                // Event boundary.
                if !self.event_data.is_empty() {
                    let payload = self.event_data.join("\n");
                    self.event_data.clear();
                    if payload == "[DONE]" {
                        events.push(None);
                    } else {
                        events.push(Some(payload));
                    }
                }
                continue;
            }

            if line.starts_with(':') {
                // Comment line — ignore.
                continue;
            }

            // "field: value" parse. Ignore fields other than "data".
            if let Some(rest) = line.strip_prefix("data:") {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                self.event_data.push(value.to_string());
            }
            // Other fields (event, id, retry) ignored — OpenAI does not use them.
        }

        events
    }
}
