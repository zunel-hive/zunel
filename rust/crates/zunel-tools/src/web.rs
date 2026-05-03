use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ssrf::validate_url_target;
use crate::tool::{Tool, ToolContext, ToolResult};
use crate::web_search_providers::{
    BraveProvider, DuckDuckGoProvider, StubProvider, WebSearchProvider,
};

pub struct WebFetchTool {
    client: reqwest::Client,
    allow_loopback: bool,
}

impl WebFetchTool {
    pub fn new() -> Self {
        // `Client::builder().build()` only fails on broken TLS / DNS
        // configurations. Fall back to the default client so a misconfigured
        // host still gets a working `web_fetch` tool (just without our
        // preferred timeout / redirect cap) instead of panicking on startup.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            allow_loopback: false,
        }
    }
    /// Test-only: allow 127.0.0.1 for wiremock-driven tests.
    pub fn for_test() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
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
        let parsed = match validate_url_target(url, self.allow_loopback, "web_fetch") {
            Ok(u) => u,
            Err(e) => return ToolResult::err(e.to_string()),
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
            // htmd is a turndown-style HTML→Markdown converter (Apache-2.0).
            // If conversion fails for some pathological markup, fall back
            // to the raw body so the agent still gets *something* useful.
            let md = htmd::convert(&body).unwrap_or_else(|_| body.clone());
            ToolResult::ok(md)
        } else {
            ToolResult::ok(body)
        }
    }
}

pub struct WebSearchTool {
    provider: Box<dyn WebSearchProvider>,
}

impl WebSearchTool {
    pub fn new(provider: Box<dyn WebSearchProvider>) -> Self {
        Self { provider }
    }

    pub fn brave(api_key: String) -> Self {
        Self::new(Box::new(BraveProvider::new(api_key)))
    }

    pub fn brave_with_endpoint(api_key: String, endpoint: String) -> Self {
        Self::new(Box::new(BraveProvider::with_endpoint(api_key, endpoint)))
    }

    pub fn duckduckgo() -> Self {
        Self::new(Box::new(DuckDuckGoProvider::new()))
    }

    pub fn stub(name: &'static str) -> Self {
        Self::new(Box::new(StubProvider {
            provider_name: name,
        }))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        "Search the web and return a short list of results (title, URL, snippet)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "n": {"type": "integer", "default": 5},
            },
            "required": ["query"],
        })
    }
    fn concurrency_safe(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return ToolResult::err("web_search: missing query".to_string());
        };
        let n = args.get("n").and_then(Value::as_u64).unwrap_or(5) as usize;
        match self.provider.search(query, n).await {
            Ok(results) => {
                let rendered: Vec<String> = results.iter().map(|r| r.render()).collect();
                ToolResult::ok(rendered.join("\n\n"))
            }
            Err(e) => ToolResult::err(e.to_string()),
        }
    }
}
