use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use claw_types::AppResult;
use crate::llm::{ChatProvider, LlmDelta, LlmRequest};
use crate::tool::Tool;

// ========================= MockProvider =========================

pub struct MockProvider {
    name: String,
    scripts: Mutex<std::collections::VecDeque<Vec<LlmDelta>>>,
}

impl MockProvider {
    pub fn new(name: impl Into<String>, scripts: Vec<Vec<LlmDelta>>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            scripts: Mutex::new(scripts.into_iter().collect()),
        })
    }

    pub fn echo_text(name: impl Into<String>, text: impl Into<String>) -> Arc<Self> {
        Self::new(
            name,
            vec![vec![LlmDelta::Text(text.into()), LlmDelta::Done]],
        )
    }
}

#[async_trait]
impl ChatProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat_stream(&self, _req: LlmRequest) -> AppResult<mpsc::Receiver<LlmDelta>> {
        let mut script = {
            let mut q = self.scripts.lock().expect("scripts mutex poisoned");
            q.pop_front().unwrap_or_else(|| vec![LlmDelta::Done])
        };

        let needs_done = !matches!(script.last(), Some(LlmDelta::Done));
        if needs_done {
            script.push(LlmDelta::Done);
        }

        let (tx, rx) = mpsc::channel::<LlmDelta>(script.len().max(1));
        tokio::spawn(async move {
            for d in script {
                if tx.send(d).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}

// ========================= MockTool =========================

pub struct MockTool {
    name: String,
    description: String,
    response: Value,
}

impl MockTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        response: Value,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            description: description.into(),
            response,
        })
    }

    pub fn echo() -> Arc<Self> {
        Self::new(
            "echo",
            "echo back the input.message field as JSON",
            json!({"echo": "<from-args>"}),
        )
    }
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        })
    }

    async fn invoke(&self, args: Value) -> AppResult<String> {
        let mut out = self.response.clone();
        if self.name == "echo" {
            if let Some(msg) = args.get("message").cloned() {
                out = json!({ "echo": msg });
            } else {
                out = json!({ "echo": args });
            }
        }
        Ok(out.to_string())
    }
}

// ========================= 自测 =========================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatMessage;
    use std::time::Duration;

    fn req() -> LlmRequest {
        LlmRequest {
            provider: "mock".into(),
            model: "m".into(),
            messages: vec![ChatMessage::user("hi")],
            temperature: 0.0,
            top_p: 1.0,
            max_tokens: 16,
            stream: true,
            timeout: Duration::from_secs(1),
            tools: vec![],
        }
    }

    #[tokio::test]
    async fn mock_provider_emits_scripted_deltas_then_done() {
        let mp = MockProvider::echo_text("mock", "hello");
        let mut rx = mp.chat_stream(req()).await.unwrap();
        let first = rx.recv().await.expect("text");
        assert!(matches!(first, LlmDelta::Text(s) if s == "hello"));
        let last = rx.recv().await.expect("done");
        assert!(matches!(last, LlmDelta::Done));
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn mock_provider_auto_appends_done_when_missing() {
        let mp = MockProvider::new("mock", vec![vec![LlmDelta::Text("x".into())]]);
        let mut rx = mp.chat_stream(req()).await.unwrap();
        rx.recv().await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), LlmDelta::Done));
    }

    #[tokio::test]
    async fn mock_provider_returns_done_only_after_scripts_drained() {
        let mp = MockProvider::new("mock", vec![vec![LlmDelta::Done]]);
        let mut rx = mp.chat_stream(req()).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), LlmDelta::Done));
        let mut rx2 = mp.chat_stream(req()).await.unwrap();
        assert!(matches!(rx2.recv().await.unwrap(), LlmDelta::Done));
    }

    #[tokio::test]
    async fn mock_tool_echo_returns_input_message() {
        let t = MockTool::echo();
        let out = t.invoke(json!({"message": "hi"})).await.unwrap();
        assert!(out.contains("\"echo\":"));
        assert!(out.contains("hi"));
    }

    #[tokio::test]
    async fn mock_tool_custom_returns_fixed_response() {
        let t = MockTool::new("const", "d", json!({"k": 42}));
        let out = t.invoke(json!({})).await.unwrap();
        assert_eq!(out, "{\"k\":42}");
    }
}
