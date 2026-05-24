//! claw-llm 配置数据结构。
//!
//! 从 claw-core 统一导入，保持向后兼容的导入路径 `claw_llm::config::*`。

pub use claw_core::config::{
    CircuitBreakerConfig, LlmConfig, LlmProviderConfig, RetryConfig,
};
