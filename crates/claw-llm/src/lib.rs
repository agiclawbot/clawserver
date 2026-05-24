//! claw-llm：OpenAI 兼容的异步 LLM 客户端与全局调用池。
//!
//! - 实现 [`claw_core::llm::ChatProvider`] trait，对引擎层透明
//! - 内部 reqwest 连接池 / SSE 流式解析 / 熔断 / 重试
//! - LLM 相关数据结构 ([`LlmProviderConfig`] / [`RetryConfig`] / [`LlmConfig`])
//!   定义在 claw-core; 本 crate 通过 `pub use claw_core::config::*` 重新导出

pub mod config;
mod client;

pub use client::{LlmClient, LlmPool};
pub use config::{CircuitBreakerConfig, LlmConfig, LlmProviderConfig, RetryConfig};
