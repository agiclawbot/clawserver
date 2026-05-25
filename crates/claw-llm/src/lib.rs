//! # claw-llm：LLM 基础服务层
//!
//! 职责：实现 `ChatProvider` trait，封装 OpenAI 兼容 HTTP 协议的流式调用。
//!
//! ## 核心组件
//!
//! | 组件 | 职责 |
//! |------|------|
//! | `LlmPool` | 全局 LLM 调用池，按 provider 名获取 `Arc<dyn ChatProvider>` |
//! | `LlmClient` | 单个 provider 的 HTTP 客户端（reqwest + 连接池 + 熔断 + 重试） |
//!
//! ## 调用链路
//!
//! ```text
//! LlmPool::get_dyn("openai")
//!   → Arc<dyn ChatProvider>
//!      → chat_stream(LlmRequest)
//!         → POST /v1/chat/completions (SSE stream)
//!         → 解析 data: 行 → LlmDelta
//!         → mpsc::Receiver<LlmDelta>
//! ```
//!
//! ## 可替换实现
//!
//! `claw_llm` 是 `ChatProvider` 的**一种**具体实现。未来可以：
//! - 用 rig crate 替换（Rust 原生 LLM 框架）
//! - 用 adk-rust 替换（Google ADK 的 Rust 实现）
//! - 对接自定义协议（gRPC、WebSocket 等）
//! 只需在 `claw-core::llm::ChatProvider` trait 下另起实现即可。

pub mod config;
mod client;

pub use client::{LlmClient, LlmPool};
pub use config::{CircuitBreakerConfig, LlmConfig, LlmProviderConfig, RetryConfig};
