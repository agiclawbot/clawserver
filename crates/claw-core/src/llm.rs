//! LLM 调用契约：[`LlmRequest`] / [`LlmDelta`] / [`ChatProvider`] trait。
//!
//! 设计：
//! - 仅 trait + 数据模型，不含具体 HTTP / SSE 实现
//! - [`ChatProvider`] 是对象安全的（dyn 兼容），引擎层只持 `Arc<dyn ChatProvider>`
//! - 流式增量统一用 `tokio::sync::mpsc::Receiver<LlmDelta>`，下游按需消费
//! - 具体实现（OpenAI 兼容客户端 / mock / rig / adk-rust 适配）位于上层 crate

use std::time::Duration;

use tokio::sync::mpsc;

use crate::chat::ChatMessage;
use crate::error::AppResult;
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
    /// 可选的工具描述（为空表示不开启 tool calling）。
    pub tools: Vec<ToolSpec>,
}

/// 流式增量输出。
#[derive(Debug, Clone)]
pub enum LlmDelta {
    /// 文本增量。
    Text(String),
    /// 完整解析后的一批 tool_calls（一次性发出，避免下游拼接负担）。
    ToolCalls(Vec<ToolCall>),
    /// 上游表示本轮结束。
    Done,
    /// 错误（建流后的中途错误才会出现这里；建流前的错误走 `Result::Err`）。
    Error(String),
}

/// LLM Provider 扩展点。
///
/// 所有 LLM 实现（OpenAI 兼容 / adk-rust / rig / 本地 mock 等）都实现此 trait。
/// 引擎层只依赖此 trait，不感知具体传输实现。
///
/// 设计约束：
/// - 纯异步、返回 `mpsc::Receiver<LlmDelta>`，下游按需消费
/// - 实现中应封装熔断 / 重试 / 超时，对上层透明
/// - 建流失败返 `Err`；建流后的错误通过 [`LlmDelta::Error`] 投递
#[async_trait::async_trait]
pub trait ChatProvider: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn chat_stream(&self, req: LlmRequest) -> AppResult<mpsc::Receiver<LlmDelta>>;
}
