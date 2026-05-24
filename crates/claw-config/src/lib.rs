//! claw-config：配置数据向后兼容的重新导出层。
//!
//! 所有配置类型与加载逻辑已迁移到 claw-core；本 crate 仅作重新导出，
//! 便于渐进式迁移。新代码应直接 `use claw_core::config::*`。

pub use claw_core::config::{
    self, init_from_dir, AppConfig, CircuitBreakerConfig, ConfigHandle, LlmConfig,
    LlmEndpoint, LlmProviderConfig, MemoryConfig, ObservabilityConfig, PromptConfig,
    QueueConfig, RateLimitConfig, RedisConfig, RetryConfig, ServerConfig, TaskConfig,
    TaskLlmConfig, TaskMode,
};
pub use claw_core::buffer::BufferConfig;
