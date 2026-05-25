//! # claw-core：ClawServer 契约层
//!
//! ## 定位
//!
//! 这是系统的**最底层**，定义"这个系统是什么"——所有跨 crate 共享的类型、trait、配置结构体
//! 都放在这里。上层 crate（claw-llm / claw-agent / claw-api）都依赖它，但 claw-core
//! **不依赖任何上层 crate**。
//!
//! 一旦稳定应尽量少改，因为任何改动都会触发全工作区重新编译。
//!
//! ## 模块清单
//!
//! | 模块 | 内容 | 变更频率 |
//! |------|------|----------|
//! | [`error`] | `AppError` / `AppResult` — 全局错误枚举 | 低 |
//! | [`chat`] | `ChatMessage` / `ChatRole` / `AssistantToolCall` — LLM 对话消息 | 低 |
//! | [`llm`] | `LlmRequest` / `LlmDelta` / `ChatProvider` trait — LLM 调用契约 | 低 |
//! | [`tool`] | `Tool` trait / `ToolCall` / `ToolRegistry` — 工具系统 | 低 |
//! | [`config`] | `AppConfig` / 所有子配置 struct / `ConfigHandle` | 中 |
//! | [`buffer`] | `BufferConfig` — 内部 channel 大小配置 | 极低 |
//! | [`util`] | `breaker::CircuitBreaker` / `retry::backoff` — 熔断与重试 | 低 |
//! | [`tools`] | 内置工具实现（TimeNow / HttpGet / WebSearch） | 中 |
//! | [`skill`] | `SkillManifest` / `SkillRegistry` / `load_from_dir` | 低 |
//! | [`test_utils`] | `MockProvider` / `MockTool` — 测试 mock 实现 | 极低 |
//!
//! ## 扩展指南
//!
//! - **新增 trait**: 在这里定义 trait，在上层 crate 实现
//! - **新增配置字段**: 在 [`config`] 模块加字段 + 默认值，YAML 自动对齐
//! - **新增内置工具**: 在 `tools::builtin` 下实现 `Tool` trait，在 `root/src/main.rs` 注册

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
