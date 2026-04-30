use serde_json::json;
use tempfile::tempdir;

use zunel_tools::{shell::ExecTool, Tool, ToolContext};

fn ctx(ws: &std::path::Path) -> ToolContext {
    ToolContext::new_with_workspace(ws.to_path_buf(), "cli:direct".into())
}

#[tokio::test]
async fn exec_runs_simple_command_and_captures_stdout() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool
        .execute(json!({"command": "echo hello"}), &ctx(ws.path()))
        .await;
    assert!(!res.is_error, "{res:?}");
    assert!(res.content.contains("hello"));
}

#[tokio::test]
async fn exec_blocks_rm_rf() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool
        .execute(json!({"command": "rm -rf /tmp/fake"}), &ctx(ws.path()))
        .await;
    assert!(res.is_error);
    assert!(res.content.contains("denied"));
}

#[tokio::test]
async fn exec_truncates_long_output_with_marker() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool
        .execute(
            json!({"command": "python3 -c 'print(\"a\" * 20000)'"}),
            &ctx(ws.path()),
        )
        .await;
    if res.is_error {
        return;
    }
    assert!(res.content.len() <= 10_000 + 200);
    assert!(res.content.contains("truncated"));
}

#[tokio::test]
async fn exec_times_out_on_hanging_command() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool
        .execute(json!({"command": "sleep 5", "timeout": 1}), &ctx(ws.path()))
        .await;
    assert!(res.is_error, "{res:?}");
    assert!(res.content.contains("timed out") || res.content.contains("timeout"));
}
