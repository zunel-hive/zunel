use std::fs;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn agent_one_shot_prints_provider_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "cc-1",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "integration ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "gpt-x" }} }}
            }}"#,
            server.uri()
        ),
    )
    .unwrap();

    let assert = Command::cargo_bin("zunel")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("agent")
        .arg("-m")
        .arg("hi")
        .assert();

    assert
        .success()
        .stdout(predicates::str::contains("integration ok"));
}
