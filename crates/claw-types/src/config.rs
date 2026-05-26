use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::buffer::BufferConfig;
use crate::error::{AppError, AppResult};

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
    pub buffer: BufferConfig,
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

#[derive(Debug, Clone, Deserialize)]
pub struct AdminConfig {
    #[serde(default = "default_admin_enabled")]
    pub enabled: bool,
    pub bind: String,
    pub api_key_env: String,
}

fn default_admin_enabled() -> bool { false }

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
    #[serde(default)]
    pub mode: TaskMode,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    #[default]
    Plain,
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
    #[serde(default)]
    pub fallback: Vec<LlmEndpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmEndpoint {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptConfig {
    #[serde(default)]
    pub system: String,
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

// ========================= 加载逻辑 =========================

#[derive(Clone)]
pub struct ConfigHandle(Arc<AppConfig>);

impl ConfigHandle {
    #[inline]
    pub fn load(&self) -> Arc<AppConfig> {
        self.0.clone()
    }
}

pub fn init_from_dir(dir: &std::path::Path) -> AppResult<ConfigHandle> {
    let main_path = dir.join("config.yaml");
    let main_raw = std::fs::read_to_string(&main_path).map_err(|e| {
        AppError::Config(format!("read {}: {}", main_path.display(), e))
    })?;
    let mut cfg: AppConfig = serde_yaml::from_str(&main_raw)?;

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
                .map_err(|e| AppError::Config(format!("read {}: {}", p.display(), e)))?;
            let tc: TaskConfig = serde_yaml::from_str(&raw)?;
            if tc.enabled {
                cfg.tasks.insert(tc.name.clone(), tc);
            }
        }
    }

    validate(&cfg)?;
    Ok(ConfigHandle(Arc::new(cfg)))
}

fn validate(cfg: &AppConfig) -> AppResult<()> {
    if cfg.server.bind.is_empty() {
        return Err(AppError::Config("server.bind empty".into()));
    }
    if cfg.redis.urls.is_empty() {
        return Err(AppError::Config("redis.urls empty".into()));
    }
    if cfg.llm.providers.is_empty() {
        return Err(AppError::Config("llm.providers empty".into()));
    }
    if !cfg.llm.providers.contains_key(&cfg.llm.default_provider) {
        return Err(AppError::Config(format!(
            "llm.default_provider `{}` not defined",
            cfg.llm.default_provider
        )));
    }
    for (name, tc) in &cfg.tasks {
        if !cfg.llm.providers.contains_key(&tc.llm.provider) {
            return Err(AppError::Config(format!(
                "task `{name}` uses unknown provider `{}`",
                tc.llm.provider
            )));
        }
        for ep in &tc.llm.fallback {
            if !cfg.llm.providers.contains_key(&ep.provider) {
                return Err(AppError::Config(format!(
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
    fn parse_app_config_from_yaml() {
        let yaml = r#"
server:
  bind: "0.0.0.0:3385"
  worker_threads: 0
  body_limit_bytes: 1048576
  request_timeout_secs: 30
  sse_keep_alive_secs: 15
  graceful_shutdown_secs: 10
  tcp_keepalive_secs: 60
  tcp_nodelay: true
rate_limit:
  enabled: false
  per_second: 100
  burst: 200
circuit_breaker:
  failure_ratio: 0.5
  min_samples: 8
  rolling_window_secs: 60
  open_duration_secs: 5
  half_open_max_probes: 2
redis:
  urls: ["redis://127.0.0.1:6379"]
  pool_size: 8
  connect_timeout_ms: 1000
  command_timeout_ms: 1000
  session_prefix: "claw:sess:"
  session_ttl_secs: 3600
  memory_max_turns: 20
buffer:
  channel_size: 256
queue:
  capacity: 128
  enqueue_timeout_ms: 1000
observability:
  log_level: "info"
  log_format: "text"
llm:
  default_provider: mock
  providers:
    mock:
      base_url: "http://localhost:9999"
      api_key_env: MOCK_API_KEY
      pool_idle_per_host: 1
      pool_max_idle_secs: 10
      request_timeout_secs: 5
      connect_timeout_secs: 2
      default_model: mock-model
  retry:
    max_attempts: 1
    base_backoff_ms: 10
    max_backoff_ms: 100
"#;
        let cfg: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.server.bind, "0.0.0.0:3385");
        assert_eq!(cfg.redis.urls.len(), 1);
        assert!(cfg.llm.providers.contains_key("mock"));
    }
}
