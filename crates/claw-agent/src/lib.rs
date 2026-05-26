//! # claw-agent：编排层
//!
//! 位于架构中间层，上接 API/CLI 边界层，下调用 LLM/Redis/Config 基础层。
//!
//! ## 模块职责
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`engine`] | `AgentEngine` — 组合所有依赖，对外提供 `run_stream()` 入口 |
//! | [`memory`] | `SessionStore` trait + `RedisSessionStore` — 会话记忆读写 |
//! | [`react`] | `run_react()` — ReAct 多轮循环（Thought→Tool→Observation） |
//! | [`task`] | `TaskRegistry` — 任务配置的只读索引（Arc 共享） |
//! | [`skill`] | `SkillRegistry` — Skill 加载与查询 |
//! | [`tools`] | 内置工具实现（TimeNow / HttpGet / WebSearch） |

pub mod engine;
pub mod memory;
pub mod react;
pub mod skill;
pub mod task;
pub mod tools;

pub use engine::{AgentEngine, AgentInput};
pub use memory::{SessionMemory, SessionStore};
pub use react::{run_react, ReactConfig, ReactEvent};
pub use task::TaskRegistry;
