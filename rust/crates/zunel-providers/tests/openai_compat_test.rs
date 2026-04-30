use std::collections::BTreeMap;

use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_providers::{ChatMessage, GenerationSettings, LLMProvider, OpenAICompatProvider, Role};

fn canned_response_body() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "hello from wiremock" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
    })
}

#[tokio::test]
async fn generates_simple_completion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer sk-test"))
        .and(body_partial_json(serde_json::json!({ "model": "gpt-x" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new("sk-test".into(), server.uri(), BTreeMap::new())
        .expect("provider builds");

    let response = provider
        .generate(
            "gpt-x",
            &[ChatMessage {
                role: Role::User,
                content: "hi".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .expect("generate ok");

    assert_eq!(response.content.as_deref(), Some("hello from wiremock"));
    assert_eq!(response.usage.prompt_tokens, 5);
    assert_eq!(response.usage.completion_tokens, 3);
    assert!(response.tool_calls.is_empty());
}

#[tokio::test]
async fn propagates_extra_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("X-Demo", "42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let mut headers = BTreeMap::new();
    headers.insert("X-Demo".into(), "42".into());
    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), headers).expect("provider builds");

    provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .expect("generate ok");
}

#[tokio::test]
async fn non_retryable_error_returns_provider_returned() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let err = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap_err();

    match err {
        zunel_providers::Error::ProviderReturned { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("bad request"));
        }
        other => panic!("expected ProviderReturned, got {other:?}"),
    }
}

#[tokio::test]
async fn request_body_matches_snapshot() {
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    struct CaptureRequest {
        captured: Arc<Mutex<Option<serde_json::Value>>>,
    }

    impl Respond for CaptureRequest {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            *self.captured.lock().unwrap() = Some(body);
            ResponseTemplate::new(200).set_body_json(canned_response_body())
        }
    }

    let captured = Arc::new(Mutex::new(None));
    let responder = CaptureRequest {
        captured: captured.clone(),
    };

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    provider
        .generate(
            "gpt-x",
            &[ChatMessage::system("be brief"), ChatMessage::user("hi")],
            &[],
            &GenerationSettings {
                temperature: Some(0.2),
                max_tokens: Some(512),
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();

    let body = captured.lock().unwrap().take().expect("request captured");
    insta::assert_json_snapshot!("openai_compat_request_body", body);
}

#[tokio::test]
async fn retries_once_on_429_then_succeeds() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string("slow down"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let response = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.content.as_deref(), Some("hello from wiremock"));
}

#[tokio::test]
async fn gives_up_after_one_retry() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string("still slow"),
        )
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let err = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, zunel_providers::Error::RateLimited { .. }));
}
