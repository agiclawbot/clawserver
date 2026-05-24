//! 通用工具：原子熔断器、重试退避算法。
//!
//! 所有结构均为无锁 / 无互斥锁设计：
//! - [`breaker::CircuitBreaker`]: 基于 AtomicU64 原子窗口计数 + CAS 状态切换
//! - [`retry::backoff`]: 纯函数式退避，不持状态

pub mod breaker;
pub mod retry;
