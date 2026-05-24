//! claw-api：HTTP API + axum 路由 + SSE + ops endpoints。
//!
//! 设计：
//! - 单 `Router` + `State(Arc<AgentEngine>)`，所有 handler 零锁只读
//! - `tower_governor` 令牌桶限流（无锁实现）
//! - `tower_http::trace` 结构化 tracing
//! - 优雅关闭：`signal::ctrl_c` + SIGTERM 双通道，drain in-flight 请求

pub mod dto;
pub mod metrics;
pub mod server;
pub mod stream;

pub use dto::{AgentRequest, AgentResponseMeta};
pub use server::{build_router, serve};
pub use stream::agent_stream;
