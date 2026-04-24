use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::mpsc;
use zunel_config::AgentDefaults;
use zunel_core::{AgentLoop, SessionManager};
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, StreamEvent, ToolSchema, Usage,
};

struct StreamingFake {
    chunks: Vec<String>,
    captured_messages: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
}

#[async_trait]
impl LLMProvider for StreamingFake {
    async fn generate(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSchema],
        _settings: &GenerationSettings,
    ) -> zunel_providers::Result<LLMResponse> {
        unreachable!("streaming path only in this test")
    }

    fn generate_stream<'a>(
        &'a self,
        _model: &'a str,
        messages: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        _settings: &'a GenerationSettings,
    ) -> BoxStream<'a, zunel_providers::Result<StreamEvent>> {
        self.captured_messages.lock().unwrap().push(messages.to_vec());
        let chunks = self.chunks.clone();
        Box::pin(async_stream::stream! {
            let mut full = String::new();
            for c in &chunks {
                full.push_str(c);
                yield Ok(StreamEvent::ContentDelta(c.clone()));
            }
            yield Ok(StreamEvent::Done(LLMResponse {
                content: Some(full),
                tool_calls: Vec::new(),
                usage: Usage { prompt_tokens: 2, completion_tokens: 3, cached_tokens: 0 },
            }));
        })
    }
}

fn make_loop(tmp: &tempfile::TempDir) -> (AgentLoop, Arc<Mutex<Vec<Vec<ChatMessage>>>>) {
    let workspace: PathBuf = tmp.path().to_path_buf();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn LLMProvider> = Arc::new(StreamingFake {
        chunks: vec!["hel".into(), "lo".into()],
        captured_messages: captured.clone(),
    });
    let manager = SessionManager::new(&workspace);
    let defaults = AgentDefaults {
        provider: Some("custom".into()),
        model: "gpt-x".into(),
        ..Default::default()
    };
    let l = AgentLoop::with_sessions(provider, defaults, manager);
    (l, captured)
}

#[tokio::test]
async fn process_streamed_emits_deltas_and_persists_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (loop_, _) = make_loop(&tmp);
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(16);

    let handle = tokio::spawn(async move {
        let mut deltas = Vec::new();
        while let Some(event) = rx.recv().await {
            if let StreamEvent::ContentDelta(s) = event {
                deltas.push(s);
            }
        }
        deltas
    });

    let result = loop_
        .process_streamed("cli:direct", "hi", tx)
        .await
        .unwrap();
    assert_eq!(result.content, "hello");

    let deltas = handle.await.unwrap();
    assert_eq!(deltas, vec!["hel", "lo"]);

    // Session must now exist on disk with user + assistant messages.
    let manager = SessionManager::new(tmp.path());
    let session = manager.load("cli:direct").unwrap().expect("saved");
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0]["role"].as_str(), Some("user"));
    assert_eq!(session.messages()[0]["content"].as_str(), Some("hi"));
    assert_eq!(session.messages()[1]["role"].as_str(), Some("assistant"));
    assert_eq!(session.messages()[1]["content"].as_str(), Some("hello"));
}

#[tokio::test]
async fn process_streamed_feeds_history_to_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let (loop_, captured) = make_loop(&tmp);

    // First turn seeds history.
    let (tx1, _rx1) = mpsc::channel::<StreamEvent>(16);
    loop_.process_streamed("cli:direct", "ping", tx1).await.unwrap();

    let (tx2, _rx2) = mpsc::channel::<StreamEvent>(16);
    loop_.process_streamed("cli:direct", "again", tx2).await.unwrap();

    let calls = captured.lock().unwrap();
    // Second call should see prior user + assistant + new user message.
    assert!(calls.len() >= 2);
    let second = &calls[1];
    assert!(second.len() >= 3, "expected ≥3 messages, got {second:?}");
    assert_eq!(second.last().unwrap().content, "again");
}
