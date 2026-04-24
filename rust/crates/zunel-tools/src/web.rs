use std::time::Duration;

use async_trait::async_trait;
use html2md::parse_html;
use serde_json::{json, Value};

use crate::ssrf::validate_url_target;
use crate::tool::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool {
    client: reqwest::Client,
    allow_loopback: bool,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .expect("reqwest client builds"),
            allow_loopback: false,
        }
    }
    /// Test-only: allow 127.0.0.1 for wiremock-driven tests.
    pub fn for_test() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            allow_loopback: true,
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch a URL and return its body. HTML is converted to markdown."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
            },
            "required": ["url"],
        })
    }
    fn concurrency_safe(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let Some(url) = args.get("url").and_then(Value::as_str) else {
            return ToolResult::err("web_fetch: missing url".to_string());
        };
        let parsed = match validate_url_target(url, self.allow_loopback) {
            Ok(u) => u,
            Err(e) => return ToolResult::err(format!("web_fetch: {e}")),
        };
        let resp = match self.client.get(parsed).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("web_fetch: request failed: {e}")),
        };
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => return ToolResult::err(format!("web_fetch: body read failed: {e}")),
        };
        if ctype.starts_with("text/html") || body.trim_start().starts_with("<!") {
            let md = parse_html(&body);
            ToolResult::ok(md)
        } else {
            ToolResult::ok(body)
        }
    }
}
