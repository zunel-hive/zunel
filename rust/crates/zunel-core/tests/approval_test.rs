use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex};

use zunel_core::approval::{
    summarize_tool_call, tool_requires_approval, ApprovalDecision, ApprovalHandler,
    ApprovalRequest, ApprovalScope, CachedApprovalHandler,
};

struct FakeHandler {
    answers: Arc<Mutex<Vec<ApprovalDecision>>>,
    calls: Arc<Mutex<Vec<ApprovalRequest>>>,
}

#[async_trait]
impl ApprovalHandler for FakeHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        self.calls.lock().unwrap().push(req);
        self.answers
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(ApprovalDecision::Deny)
    }
}

#[tokio::test]
async fn summarize_tool_call_is_stable_across_arg_orders() {
    let a = summarize_tool_call("exec", &json!({"command": "ls", "timeout": 60}));
    let b = summarize_tool_call("exec", &json!({"timeout": 60, "command": "ls"}));
    assert_eq!(a, b);
}

#[test]
fn tool_requires_approval_respects_scope() {
    assert!(tool_requires_approval("exec", ApprovalScope::Shell));
    assert!(!tool_requires_approval("read_file", ApprovalScope::Shell));
    assert!(tool_requires_approval("write_file", ApprovalScope::Writes));
    assert!(!tool_requires_approval("read_file", ApprovalScope::Writes));
    assert!(tool_requires_approval("read_file", ApprovalScope::All));
}

#[tokio::test]
async fn cached_handler_only_prompts_once_per_tool_call_signature() {
    let inner = FakeHandler {
        answers: Arc::new(Mutex::new(vec![
            ApprovalDecision::Approve,
            ApprovalDecision::Approve,
        ])),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls = inner.calls.clone();
    let cached = CachedApprovalHandler::new(Arc::new(inner));
    let req1 = ApprovalRequest {
        tool_name: "exec".into(),
        args: json!({"command": "ls"}),
        description: "list".into(),
        scope: ApprovalScope::Shell,
    };
    let req2 = req1.clone();

    assert!(matches!(
        cached.request(req1).await,
        ApprovalDecision::Approve
    ));
    assert!(matches!(
        cached.request(req2).await,
        ApprovalDecision::Approve
    ));

    assert_eq!(calls.lock().unwrap().len(), 1);
}
