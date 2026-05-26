//! # claw-api：HTTP 边界层
//!
//! 架构最外层，直接面向客户端。职责：协议转换，不包含业务逻辑。
//!
//! ## 模块职责
//!
//! | 模块 | 职责 | 关键路径 |
//! |------|------|----------|
//! | [`server`] | axum Router 构建 + TCP 监听 + 优雅关闭 | `build_router()` / `serve()` |
//! | [`stream`] | POST `/v1/agent/stream` SSE 流处理器 | `agent_stream()` |
//! | [`dto`] | 请求/响应 DTO + `#[serde(deny_unknown_fields)]` 校验 | `AgentRequest` |
//! | [`metrics`] | Prometheus 指标定义 + axum middleware | `/metrics` |
//!
//! ## 数据流
//!
//! ```text
//! HTTP POST /v1/agent/stream  →  Json<AgentRequest>  →  validate()
//!     →  AgentEngine::run_stream(input)  →  LlmDelta 流  →  SSE 事件
//! ```
//!
//! ## 设计约束
//!
//! - 单 `Router` + `State(Arc<AgentEngine>)`，所有 handler 零锁只读
//! - 请求体校验在 DTO 层完成（长度限制 + 字段白名单），不会把脏数据传入引擎

pub mod dto;
pub mod error;
pub mod metrics;
pub mod server;
pub mod stream;

pub use dto::{AgentRequest, AgentResponseMeta};
pub use server::{build_router, serve};
pub use stream::agent_stream;
