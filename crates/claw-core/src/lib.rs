//! claw-core：ClawServer 的"语言契约层"。
//!
//! # 定位
//! - **零业务依赖**：只放跨 crate 共享的纯数据模型与 trait
//! - **最稳定**：所有上层 crate 都依赖它，本 crate 一旦稳定不应频繁改动
//! - **trait 主导**：用 trait 抽象 LLM、工具、记忆等可替换组件
//!
//! # 模块（按计划逐步填充）
//! - [`error`]    全局错误模型（Step 1：从 clawserver/src/error.rs 迁入纯枚举）
//! - [`chat`]     ChatMessage / ChatRole / AssistantToolCall（Step 2）
//! - [`llm`]      LlmRequest / LlmDelta / ChatProvider trait（Step 2）
//! - [`tool`]     Tool trait / ToolCall / ToolResult / ToolSpec（Step 3）
//! - [`util`]     无锁熔断器 / 重试策略（Step 1 后期）
//!
//! # 当前状态
//! 骨架阶段，所有模块为占位符；逐步从根 crate 搬运代码并切换 use 路径。

pub mod buffer;
pub mod config;
pub mod error;
pub mod tool;
pub mod tools;
pub mod chat;
pub mod llm;

pub mod util;

pub mod skill;

pub mod test_utils;

// 顶层别名：让下游 `use claw_core::{AppError, AppResult};` 更简洁。
pub use error::{AppError, AppResult};
