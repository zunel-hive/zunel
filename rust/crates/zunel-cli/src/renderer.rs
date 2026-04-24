// `#![allow(dead_code)]` is lifted in Task 11 when the one-shot
// `agent -m` command wires `StreamingRenderer::start().drive(rx)`.
// Kept here so this commit stays clippy-clean in isolation.
#![allow(dead_code)]

use std::io::{self, Write};

use tokio::sync::mpsc::Receiver;
use zunel_providers::StreamEvent;

use crate::spinner::ThinkingSpinner;

/// Consumes a stream event channel and writes assistant output to stdout
/// as it arrives. Slice 2 renders plain text; markdown rendering lands
/// in slice 3 and can replace this module in place.
pub struct StreamingRenderer {
    spinner: Option<ThinkingSpinner>,
    header_printed: bool,
    wrote_anything: bool,
}

impl StreamingRenderer {
    pub fn start() -> Self {
        Self {
            spinner: Some(ThinkingSpinner::start()),
            header_printed: false,
            wrote_anything: false,
        }
    }

    pub async fn drive(mut self, mut rx: Receiver<StreamEvent>) -> io::Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    if !self.header_printed {
                        if let Some(spinner) = self.spinner.take() {
                            spinner.stop().await;
                        }
                        writeln!(handle, "\nzunel:")?;
                        self.header_printed = true;
                    }
                    handle.write_all(text.as_bytes())?;
                    handle.flush()?;
                    self.wrote_anything = true;
                }
                StreamEvent::Done(_) => {
                    if self.wrote_anything {
                        writeln!(handle)?;
                    }
                }
            }
        }

        if let Some(spinner) = self.spinner.take() {
            spinner.stop().await;
        }
        Ok(())
    }
}
