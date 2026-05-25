//! # claw-agent：编排层
//!
//! 位于架构中间层，上接 API/CLI 边界层，下调用 LLM/Redis 基础层。
//!
//! ## 模块职责
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`engine`] | `AgentEngine` — 组合所有依赖，对外提供 `run_stream()` 入口 |
//! | [`memory`] | `SessionStore` trait + `RedisSessionStore` — 会话记忆读写 |
//! | [`react`] | `run_react()` — ReAct 多轮循环（Thought→Tool→Observation） |
//! | [`task`] | `TaskRegistry` — 任务配置的只读索引（Arc 共享） |
//!
//! ## 外部依赖
//!
//! - 配置来自 `claw_config`（实际通过 `claw_core::config`）
//! - LLM 调用委托给 `claw_llm::LlmPool`
//! - Session 存储委托给 `fred::RedisPool`
//!
//! ## 无锁设计
//!
//! - `AgentEngine` 内所有字段 `Arc<...>`，运行期只读
//! - 可变状态（会话）下沉 Redis，进程内零共享可变状态
//! - 配置通过 `ConfigHandle` O(1) 原子加载

pub mod engine;
pub mod memory;
pub mod react;
pub mod task;

pub use engine::{AgentEngine, AgentInput};
pub use memory::{SessionMemory, SessionStore};
pub use react::{run_react, ReactConfig, ReactEvent};
pub use task::TaskRegistry;
