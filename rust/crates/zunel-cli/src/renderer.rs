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
        let mut stdout = io::stdout();

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
                        writeln!(stdout, "\nzunel:")?;
                        self.header_printed = true;
                    }
                    stdout.write_all(text.as_bytes())?;
                    stdout.flush()?;
                    self.wrote_anything = true;
                }
                StreamEvent::Done(_) => {
                    if self.wrote_anything {
                        writeln!(stdout)?;
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
