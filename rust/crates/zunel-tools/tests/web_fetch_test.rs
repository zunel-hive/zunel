use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use zunel_tools::{web::WebFetchTool, Tool, ToolContext};

#[tokio::test]
async fn web_fetch_returns_markdown_of_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/doc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><h1>Title</h1><p>body text</p></body></html>")
                .insert_header("content-type", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::for_test();
    let url = format!("{}/doc", server.uri());
    let res = tool
        .execute(json!({"url": url}), &ToolContext::for_test())
        .await;
    assert!(!res.is_error, "{res:?}");
    assert!(res.content.contains("Title"));
    assert!(res.content.contains("body text"));
}

#[tokio::test]
async fn web_fetch_rejects_loopback_when_ssrf_enabled() {
    let tool = WebFetchTool::new();
    let res = tool
        .execute(
            json!({"url": "http://127.0.0.1:65432/blocked"}),
            &ToolContext::for_test(),
        )
        .await;
    assert!(res.is_error);
    assert!(res.content.to_lowercase().contains("ssrf") || res.content.contains("loopback"));
}
