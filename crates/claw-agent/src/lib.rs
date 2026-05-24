//! claw-agent：Agent 引擎 + 会话记忆 + 任务注册表 + ReAct 循环。
//!
//! 设计参考 `adk-rust` 的「Agent -> Tool -> LLM」抽象，但针对 10w 并发做了
//! 裁剪与重写：
//! - 无锁：记忆下沉 Redis，进程内零共享可变状态
//! - 流式：Agent 直接返回 `mpsc::Receiver<LlmDelta>`，零缓冲复制
//! - 组合：通过 `TaskRegistry` 无锁索引配置，一次加载、多次只读

pub mod engine;
pub mod memory;
pub mod react;
pub mod task;

pub use engine::{AgentEngine, AgentInput};
pub use memory::{SessionMemory, SessionStore};
pub use react::{run_react, ReactConfig, ReactEvent};
pub use task::TaskRegistry;
