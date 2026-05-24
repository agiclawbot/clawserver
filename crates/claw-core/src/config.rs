//! ClawServer 配置数据结构与加载。
//!
//! 分层说明：
//! - **类型定义**（无条件编译）：AppConfig / ServerConfig / 子 config struct / TaskConfig 等
//! - **加载逻辑**（`#[cfg(feature = "yaml")]`）：init_from_dir / ConfigHandle / 校验
//! - 热重载已移除，ConfigHandle 只是 `Arc<AppConfig>` 的轻量包装

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ========================= 电路熔断器 =========================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CircuitBreakerConfig {
    pub failure_ratio: f64,
    pub min_samples: u32,
    pub rolling_window_secs: u64,
    pub open_duration_secs: u64,
    pub half_open_max_probes: u32,
}

// ========================= LLM 配置 =========================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmProviderConfig {
    pub base_url: String,
    pub api_key_env: String,
    pub pool_idle_per_host: usize,
    pub pool_max_idle_secs: u64,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    pub default_model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub default_provider: String,
    pub providers: HashMap<String, LlmProviderConfig>,
    pub retry: RetryConfig,
}

// ========================= 主配置 =========================

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub rate_limit: RateLimitConfig,
    pub circuit_breaker: CircuitBreakerConfig,
    pub redis: RedisConfig,
    pub llm: LlmConfig,
    pub queue: QueueConfig,
    pub buffer: crate::buffer::BufferConfig,
    pub observability: ObservabilityConfig,

    /// 运行期装配：task_type -> TaskConfig（从 config/tasks/*.yaml 合并）
    #[serde(default)]
    pub tasks: HashMap<String, TaskConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    #[serde(default)]
    pub worker_threads: usize,
    pub body_limit_bytes: usize,
    pub request_timeout_secs: u64,
    pub sse_keep_alive_secs: u64,
    pub graceful_shutdown_secs: u64,
    pub tcp_keepalive_secs: u64,
    pub tcp_nodelay: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub per_second: u32,
    pub burst: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub urls: Vec<String>,
    pub pool_size: usize,
    pub connect_timeout_ms: u64,
    pub command_timeout_ms: u64,
    pub session_prefix: String,
    pub session_ttl_secs: u64,
    pub memory_max_turns: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    pub capacity: usize,
    pub enqueue_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConfig {
    pub log_level: String,
    pub log_format: String,
}

// ========================= 任务配置 =========================

#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub llm: TaskLlmConfig,
    pub prompt: PromptConfig,
    pub memory: MemoryConfig,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// 运行模式：plain 是单次 LLM 流式（默认），react 是带工具的多轮循环。
    #[serde(default)]
    pub mode: TaskMode,

    /// 本任务可调用的工具名单体（mode=react 下生效）。
    #[serde(default)]
    pub tools: Vec<String>,

    /// 可选绑定某个 Skill（服务启动后从 config/skills/ 加载）。
    #[serde(default)]
    pub skill: Option<String>,

    /// ReAct 循环上限（仅 mode=react 下生效）。
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

/// 任务运行模式。
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    /// 默认：单次 LLM 流式调用，零额外开销。
    #[default]
    Plain,
    /// ReAct：带工具的多轮循环（思考 → 调工具 → 观察 → 再思考）。
    React,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskLlmConfig {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_temp")]
    pub temperature: f32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_true")]
    pub stream: bool,
    /// 多模型兜底链：主 provider 熔断 / 建立连接失败时，
    /// 按顺序尝试 fallback 列表。一旦已建立流则不再切换。
    #[serde(default)]
    pub fallback: Vec<LlmEndpoint>,
}

/// 简化的 provider+model 对，用于 fallback 链。
#[derive(Debug, Clone, Deserialize)]
pub struct LlmEndpoint {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptConfig {
    #[serde(default)]
    pub system: String,
    /// 支持 `{{content}}` 占位
    pub user_template: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub max_turns: usize,
}

// ========================= 默认值辅助 =========================

fn default_true() -> bool { true }
fn default_timeout() -> u64 { 60 }
fn default_temp() -> f32 { 0.7 }
fn default_top_p() -> f32 { 0.9 }
fn default_max_tokens() -> u32 { 2048 }
fn default_max_iterations() -> u32 { 5 }

// ========================= 加载逻辑 (yaml feature) =========================

/// 配置只读快照。
#[derive(Clone)]
pub struct ConfigHandle(Arc<AppConfig>);

impl ConfigHandle {
    /// 获取当前配置快照（O(1) 原子加载）。
    #[inline]
    pub fn load(&self) -> Arc<AppConfig> {
        self.0.clone()
    }
}

/// 从磁盘目录加载配置。
///
/// 1. 读取 `config.yaml`
/// 2. 扫描 `tasks/*.yaml` 合并入 `AppConfig.tasks`
/// 3. 校验并返回 `ConfigHandle`
#[cfg(feature = "yaml")]
pub fn init_from_dir(dir: &std::path::Path) -> crate::error::AppResult<ConfigHandle> {
    let main_path = dir.join("config.yaml");
    let main_raw = std::fs::read_to_string(&main_path).map_err(|e| {
        crate::error::AppError::Config(format!("read {}: {}", main_path.display(), e))
    })?;
    let mut cfg: AppConfig = serde_yaml::from_str(&main_raw)?;

    // 扫描 tasks 目录
    let tasks_dir = dir.join("tasks");
    if tasks_dir.is_dir() {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&tasks_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x, "yaml" | "yml"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        for p in entries {
            let raw = std::fs::read_to_string(&p)
                .map_err(|e| crate::error::AppError::Config(format!("read {}: {}", p.display(), e)))?;
            let tc: TaskConfig = serde_yaml::from_str(&raw)?;
            if tc.enabled {
                cfg.tasks.insert(tc.name.clone(), tc);
            }
        }
    }

    validate(&cfg)?;
    Ok(ConfigHandle(Arc::new(cfg)))
}

#[cfg(feature = "yaml")]
fn validate(cfg: &AppConfig) -> crate::error::AppResult<()> {
    if cfg.server.bind.is_empty() {
        return Err(crate::error::AppError::Config("server.bind empty".into()));
    }
    if cfg.redis.urls.is_empty() {
        return Err(crate::error::AppError::Config("redis.urls empty".into()));
    }
    if cfg.llm.providers.is_empty() {
        return Err(crate::error::AppError::Config("llm.providers empty".into()));
    }
    if !cfg.llm.providers.contains_key(&cfg.llm.default_provider) {
        return Err(crate::error::AppError::Config(format!(
            "llm.default_provider `{}` not defined",
            cfg.llm.default_provider
        )));
    }
    for (name, tc) in &cfg.tasks {
        if !cfg.llm.providers.contains_key(&tc.llm.provider) {
            return Err(crate::error::AppError::Config(format!(
                "task `{name}` uses unknown provider `{}`",
                tc.llm.provider
            )));
        }
        for ep in &tc.llm.fallback {
            if !cfg.llm.providers.contains_key(&ep.provider) {
                return Err(crate::error::AppError::Config(format!(
                    "task `{name}` fallback provider `{}` not defined",
                    ep.provider
                )));
            }
        }
    }
    Ok(())
}

// ========================= 测试 =========================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_mode_default_is_plain() {
        assert_eq!(TaskMode::default(), TaskMode::Plain);
    }
}

#[cfg(all(test, feature = "yaml"))]
mod yaml_tests {
    use super::*;

    #[test]
    fn task_mode_serde_round_trip() {
        let y = "plain";
        let m: TaskMode = serde_yaml::from_str(y).unwrap();
        assert_eq!(m, TaskMode::Plain);

        let y = "react";
        let m: TaskMode = serde_yaml::from_str(y).unwrap();
        assert_eq!(m, TaskMode::React);
    }

    #[test]
    fn circuit_breaker_default_fields() {
        let yaml = r#"
failure_ratio: 0.5
min_samples: 10
rolling_window_secs: 60
open_duration_secs: 30
half_open_max_probes: 3
"#;
        let c: CircuitBreakerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(c.failure_ratio, 0.5);
        assert_eq!(c.min_samples, 10);
    }
}
