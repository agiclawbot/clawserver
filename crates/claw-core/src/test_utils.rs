//! 测试用 mock 实现：在 `test-utils` feature 下暴露给下游 crate 用于 unit / integration 测试。
//!
//! - [`MockProvider`]：实现 [`crate::llm::ChatProvider`]，按预编排的 `Vec<LlmDelta>` 顺序投递
//! - [`MockTool`]：实现 [`crate::tool::Tool`]，按 name 返回固定字符串或自定义闭包
//!
//! 这两个类型纯进程内、零外部依赖，适合在 CI 或本地快速验证 Agent / 引擎逻辑。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::error::AppResult;
use crate::llm::{ChatProvider, LlmDelta, LlmRequest};
use crate::tool::Tool;

// ---------------------------------------------------------------------------
// MockProvider
// ---------------------------------------------------------------------------

/// 顺序投递预编排 deltas 的 LLM Provider，用于 ReAct 循环 / 流式解析的测试。
///
/// 每次 `chat_stream` 调用：
/// 1. 取走 `scripts` 队首一组 deltas
/// 2. 在新 task 中按顺序 send 进 mpsc::Receiver
/// 3. 末尾自动追加一条 `LlmDelta::Done`（除非编排里已含 Done）
///
/// `scripts` 用尽后会一直返回只含 `Done` 的空流，避免下游死等。
pub struct MockProvider {
    name: String,
    scripts: Mutex<std::collections::VecDeque<Vec<LlmDelta>>>,
}

impl MockProvider {
    /// 构造一个固定名 + 编排好脚本的 mock provider。
    pub fn new(name: impl Into<String>, scripts: Vec<Vec<LlmDelta>>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            scripts: Mutex::new(scripts.into_iter().collect()),
        })
    }

    /// 单帧便捷：每次调用只发一段文本然后 Done。
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

        // 保证以 Done 结尾
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

// ---------------------------------------------------------------------------
// MockTool
// ---------------------------------------------------------------------------

/// 简单 Mock 工具：按构造时提供的固定 JSON 返回；schema 为空对象。
///
/// 适合 ToolRegistry 的注册 / 派发 / 返回拼接路径测试。
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

    /// 便捷：返回 `{"echo": <input.message>}`，常见 Echo 测试。
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
        // echo 行为：把 args 合并进固定 response，便于断言
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

// ---------------------------------------------------------------------------
// 自测：MockProvider / MockTool 的基本行为
// ---------------------------------------------------------------------------
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
        rx.recv().await.unwrap(); // text
        assert!(matches!(rx.recv().await.unwrap(), LlmDelta::Done));
    }

    #[tokio::test]
    async fn mock_provider_returns_done_only_after_scripts_drained() {
        let mp = MockProvider::new("mock", vec![vec![LlmDelta::Done]]);
        let mut rx = mp.chat_stream(req()).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), LlmDelta::Done));
        // 后续调用仍可拿到只含 Done 的空流
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
