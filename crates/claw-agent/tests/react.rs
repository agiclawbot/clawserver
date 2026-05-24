//! ReAct 循环行为：用 MockProvider 模拟 LLM，断言事件序列。

use std::sync::Arc;
use std::time::Duration;

use claw_agent::{run_react, ReactConfig, ReactEvent};
use claw_core::chat::ChatMessage;
use claw_core::llm::LlmDelta;
use claw_core::test_utils::{MockProvider, MockTool};
use claw_core::tool::{ToolCall, ToolRegistry};
use serde_json::json;
use tokio::sync::mpsc::Receiver;

fn cfg(max_iter: u32) -> ReactConfig {
    ReactConfig {
        max_iterations: max_iter,
        temperature: 0.0,
        top_p: 1.0,
        max_tokens: 64,
        timeout: Duration::from_secs(2),
        provider: "mock".into(),
        model: "m".into(),
    }
}

fn registry_with_echo() -> Arc<ToolRegistry> {
    let mut r = ToolRegistry::new();
    r.register(MockTool::echo());
    Arc::new(r)
}

async fn collect(mut rx: Receiver<ReactEvent>) -> Vec<ReactEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        let is_done = matches!(ev, ReactEvent::Done);
        out.push(ev);
        if is_done {
            break;
        }
    }
    out
}

#[tokio::test]
async fn text_only_round_emits_text_then_done() {
    let llm = MockProvider::echo_text("mock", "hello world");
    let tools = registry_with_echo();

    let rx = run_react(
        llm,
        tools,
        Vec::new(),
        vec![ChatMessage::user("hi")],
        cfg(3),
        256,
    );
    let events = collect(rx).await;

    let texts: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            ReactEvent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.join(""), "hello world");
    assert!(matches!(events.last().unwrap(), ReactEvent::Done));
    // text-only 路径不应触发任何 tool 事件
    assert!(!events
        .iter()
        .any(|e| matches!(e, ReactEvent::ToolCall(_) | ReactEvent::ToolResult(_))));
}

#[tokio::test]
async fn tool_call_round_then_final_text() {
    // 第 1 轮：返回一条 tool_calls(echo, {"message":"ping"})
    // 第 2 轮：返回最终文本 "answered: ping"
    let calls = vec![ToolCall {
        id: "call_1".into(),
        name: "echo".into(),
        arguments: json!({"message": "ping"}),
    }];
    let llm = MockProvider::new(
        "mock",
        vec![
            vec![LlmDelta::ToolCalls(calls), LlmDelta::Done],
            vec![LlmDelta::Text("answered: ping".into()), LlmDelta::Done],
        ],
    );

    let rx = run_react(
        llm,
        registry_with_echo(),
        Vec::new(),
        vec![ChatMessage::user("ping me")],
        cfg(3),
        256,
    );
    let events = collect(rx).await;

    // 期望事件序列至少含：ToolCall(echo) → ToolResult(echo, ok) → Text(...) → Done
    let mut saw_tool_call = false;
    let mut saw_tool_result = false;
    let mut final_text = String::new();
    for ev in &events {
        match ev {
            ReactEvent::ToolCall(c) => {
                assert_eq!(c.name, "echo");
                assert_eq!(c.id, "call_1");
                saw_tool_call = true;
            }
            ReactEvent::ToolResult(r) => {
                assert_eq!(r.name, "echo");
                assert_eq!(r.call_id, "call_1");
                assert!(!r.is_error);
                assert!(r.content.contains("ping"));
                saw_tool_result = true;
            }
            ReactEvent::Text(t) => final_text.push_str(t),
            _ => {}
        }
    }
    assert!(saw_tool_call, "missing ToolCall event");
    assert!(saw_tool_result, "missing ToolResult event");
    assert_eq!(final_text, "answered: ping");
    assert!(matches!(events.last().unwrap(), ReactEvent::Done));
}

#[tokio::test]
async fn unknown_tool_yields_error_result_but_loop_continues() {
    let calls = vec![ToolCall {
        id: "x".into(),
        name: "ghost".into(),
        arguments: json!({}),
    }];
    let llm = MockProvider::new(
        "mock",
        vec![
            vec![LlmDelta::ToolCalls(calls), LlmDelta::Done],
            vec![LlmDelta::Text("done".into()), LlmDelta::Done],
        ],
    );

    let rx = run_react(
        llm,
        registry_with_echo(),
        Vec::new(),
        vec![ChatMessage::user("call ghost")],
        cfg(3),
        256,
    );
    let events = collect(rx).await;

    // 找到 ToolResult 必须 is_error=true
    let tool_result = events
        .iter()
        .find_map(|e| match e {
            ReactEvent::ToolResult(r) => Some(r),
            _ => None,
        })
        .expect("expected a ToolResult event");
    assert!(tool_result.is_error);
    assert!(tool_result.content.contains("not registered"));
    // 第 2 轮的最终文本仍能到达
    assert!(events
        .iter()
        .any(|e| matches!(e, ReactEvent::Text(t) if t == "done")));
}

#[tokio::test]
async fn max_iterations_exhausted_emits_error() {
    // max_iter=2；两轮都让 LLM 强行发 ToolCalls，循环用尽
    let calls = || vec![ToolCall {
        id: "loop".into(),
        name: "echo".into(),
        arguments: json!({"message": "again"}),
    }];
    let llm = MockProvider::new(
        "mock",
        vec![
            vec![LlmDelta::ToolCalls(calls()), LlmDelta::Done],
            vec![LlmDelta::ToolCalls(calls()), LlmDelta::Done],
        ],
    );

    let rx = run_react(
        llm,
        registry_with_echo(),
        Vec::new(),
        vec![ChatMessage::user("loop")],
        cfg(2),
        256,
    );
    let events = collect(rx).await;

    let err = events.iter().find_map(|e| match e {
        ReactEvent::Error(s) => Some(s.as_str()),
        _ => None,
    });
    assert!(err.is_some(), "expected Error event, got {:?}", events);
    assert!(err.unwrap().contains("max_iterations"));
    assert!(matches!(events.last().unwrap(), ReactEvent::Done));
}
