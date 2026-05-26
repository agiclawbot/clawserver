//! AppConfig YAML 解析 + ConfigHandle 句柄行为。
//!
//! 注意：claw-config 的 `init_from_dir` 会写入进程级 `OnceCell`，
//! 因此 ConfigHandle 测试只能验证一次"首次初始化"路径；
//! 多用例场景下用 serde_yaml 直解 AppConfig 更稳健。

use std::fs;

use claw_types::{init_from_dir, AppConfig, TaskMode};
use tempfile::tempdir;

const MAIN_YAML: &str = r#"
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
  retry:
    max_attempts: 3
    base_backoff_ms: 100
    max_backoff_ms: 2000

buffer:
  channel_size: 256

queue:
  capacity: 1024
  enqueue_timeout_ms: 50

observability:
  log_level: info
  log_format: json
"#;

const TASK_YAML: &str = r#"
name: chat
description: "demo chat"
enabled: true
llm:
  provider: openai
  model: "gpt-4o-mini"
prompt:
  system: "you are helpful"
  user_template: "{{content}}"
memory:
  enabled: true
  max_turns: 8
"#;

#[test]
fn task_mode_default_is_plain() {
    assert_eq!(TaskMode::default(), TaskMode::Plain);
}

#[test]
fn task_mode_serde_react_lowercase() {
    let m: TaskMode = serde_yaml::from_str("react").unwrap();
    assert_eq!(m, TaskMode::React);
    let m: TaskMode = serde_yaml::from_str("plain").unwrap();
    assert_eq!(m, TaskMode::Plain);
}

#[test]
fn parse_app_config_from_yaml() {
    let cfg: AppConfig = serde_yaml::from_str(MAIN_YAML).expect("parse");
    assert_eq!(cfg.server.bind, "0.0.0.0:3385");
    assert!(cfg.llm.providers.contains_key("openai"));
    assert_eq!(cfg.llm.default_provider, "openai");
    assert_eq!(cfg.llm.retry.max_attempts, 3);
    assert!(cfg.tasks.is_empty());
}

#[test]
fn init_from_dir_loads_main_and_tasks() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("config.yaml"), MAIN_YAML).unwrap();
    fs::create_dir(dir.path().join("tasks")).unwrap();
    fs::write(dir.path().join("tasks").join("chat.yaml"), TASK_YAML).unwrap();

    let handle = init_from_dir(dir.path()).expect("init config");
    let cfg = handle.load();
    assert_eq!(cfg.tasks.len(), 1);
    let t = cfg.tasks.get("chat").expect("chat task loaded");
    assert_eq!(t.llm.provider, "openai");
    assert_eq!(t.mode, TaskMode::Plain); // 默认值
    assert_eq!(t.max_iterations, 5); // default_max_iterations
    assert!(t.tools.is_empty());

    // 二次 init 应原子替换（幂等）
    let handle2 = init_from_dir(dir.path()).expect("re-init config");
    assert_eq!(handle2.load().tasks.len(), 1);
}

#[test]
fn init_from_dir_rejects_missing_provider() {
    let dir = tempdir().unwrap();
    let bad_main = MAIN_YAML.replace("default_provider: openai", "default_provider: ghost");
    fs::write(dir.path().join("config.yaml"), bad_main).unwrap();

    // 注意：本进程已被前一个测试初始化过 GLOBAL，但 init_from_dir 在 validate
    // 阶段就会先返回 Err，不会污染 GLOBAL。
    let res = init_from_dir(dir.path());
    let err = match res {
        Ok(_) => panic!("expected validation error"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("default_provider"));
}
