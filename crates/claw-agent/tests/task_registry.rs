//! TaskRegistry 行为：从 AppConfig 构建只读索引。

use claw_agent::TaskRegistry;
use claw_config::AppConfig;

const MAIN_YAML: &str = r#"
server:
  bind: "0.0.0.0:8080"
  worker_threads: 0
  body_limit_bytes: 1048576
  request_timeout_secs: 30
  sse_keep_alive_secs: 15
  graceful_shutdown_secs: 10
  tcp_keepalive_secs: 60
  tcp_nodelay: true
rate_limit: { enabled: false, per_second: 100, burst: 200 }
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
llm:
  default_provider: openai
  providers:
    openai:
      base_url: "https://api.openai.com"
      api_key_env: OPENAI_API_KEY
      pool_idle_per_host: 8
      pool_max_idle_secs: 30
      request_timeout_secs: 30
      connect_timeout_secs: 5
      default_model: "gpt-4o-mini"
  retry: { max_attempts: 3, base_backoff_ms: 100, max_backoff_ms: 2000 }
buffer: { channel_size: 256 }
queue: { capacity: 1024, enqueue_timeout_ms: 50 }
observability: { log_level: info, log_format: json }
tasks:
  chat:
    name: chat
    enabled: true
    llm: { provider: openai, model: "gpt-4o-mini" }
    prompt: { system: "", user_template: "{{content}}" }
    memory: { enabled: false, max_turns: 0 }
  summarize:
    name: summarize
    enabled: true
    llm: { provider: openai, model: "gpt-4o-mini" }
    prompt: { system: "", user_template: "{{content}}" }
    memory: { enabled: false, max_turns: 0 }
"#;

#[test]
fn build_indexes_all_tasks() {
    let cfg: AppConfig = serde_yaml::from_str(MAIN_YAML).unwrap();
    let reg = TaskRegistry::build(&cfg);
    assert_eq!(reg.len(), 2);
    assert!(!reg.is_empty());
    assert!(reg.contains("chat"));
    assert!(reg.contains("summarize"));
    assert!(!reg.contains("ghost"));

    let names: std::collections::HashSet<_> = reg.names().collect();
    assert!(names.contains("chat"));
    assert!(names.contains("summarize"));
}

#[test]
fn get_returns_arc_clone_per_call() {
    let cfg: AppConfig = serde_yaml::from_str(MAIN_YAML).unwrap();
    let reg = TaskRegistry::build(&cfg);
    let a = reg.get("chat").unwrap();
    let b = reg.get("chat").unwrap();
    // 都来自同一个 Arc
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    assert_eq!(a.name, "chat");
    assert!(reg.get("ghost").is_none());
}
