use std::time::Duration;

use tokio::sync::mpsc;

use crate::chat::ChatMessage;
use claw_types::AppResult;
use crate::tool::{ToolCall, ToolSpec};

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub provider: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub stream: bool,
    pub timeout: Duration,
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone)]
pub enum LlmDelta {
    Text(String),
    ToolCalls(Vec<ToolCall>),
    Done,
    Error(String),
}

#[async_trait::async_trait]
pub trait ChatProvider: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn chat_stream(&self, req: LlmRequest) -> AppResult<mpsc::Receiver<LlmDelta>>;
}
