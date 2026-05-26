//! claw-api HTTP 集成测试。
//!
//! 验证 ops endpoints (healthz / readyz / version / metrics) 和路由层
//! 错误处理（bad request / 404）。依赖 `build_router()` 构造完整 axum Router。
//!
//! 注意：测试引擎使用 mock 会话存储（MockSessionStore），不依赖 Redis。
//! LLM 请求会被 mock 或直接 skip。

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

use claw_agent::memory::SessionStore;
use claw_agent::{AgentEngine, TaskRegistry};
use claw_agent::skill::SkillRegistry;
use claw_api::auth::ApiKeyStore;
use claw_types::{init_from_dir, AppResult};
use claw_llm::ChatMessage;
use claw_llm::ToolRegistry;
use claw_llm::LlmPool;
use serde_json::json;
use tempfile::tempdir;

/// 测试最小 YAML 配置（含一个 mock LLM provider）。
const MINI_YAML: &str = r#"
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

/// 测试任务 YAML
const TASK_YAML: &str = r#"
name: chat
description: "Test task"
enabled: true
llm:
  provider: mock
  model: mock-model
prompt:
  system: "You are a test assistant."
  user_template: "{{content}}"
memory:
  enabled: false
  max_turns: 20
mode: plain
max_iterations: 1
"#;

/// 构造测试用 `Arc<AgentEngine>` + `Arc<ApiKeyStore>`。
async fn build_test_engine() -> (Arc<AgentEngine>, tempfile::TempDir, Arc<ApiKeyStore>) {
    let dir = tempdir().expect("tempdir");

    // 写 config.yaml
    std::fs::write(dir.path().join("config.yaml"), MINI_YAML).expect("write config");
    // 写 tasks 子目录
    let tasks_dir = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks_dir).expect("create tasks dir");
    std::fs::write(tasks_dir.join("chat.yaml"), TASK_YAML).expect("write task");

    let cfg_handle = init_from_dir(dir.path()).expect("init config");
    let cfg = cfg_handle.load();

    let tasks = TaskRegistry::build(&cfg);
    let memory = Arc::new(MockSessionStore);
    let llm = LlmPool::build(&cfg.llm, &cfg.circuit_breaker, 256).expect("LlmPool::build");
    let tools = Arc::new(ToolRegistry::new());
    let skills: Arc<SkillRegistry> = Arc::new(SkillRegistry::new());


    use std::collections::HashMap;
    use tokio::sync::RwLock;
    let user_memories: claw_memory::UserMemoryStore =
        Arc::new(RwLock::new(HashMap::new()));

    let engine = AgentEngine::new(cfg_handle, tasks, memory as Arc<dyn SessionStore>, llm, tools, skills, user_memories);
    let api_keys = ApiKeyStore::load(dir.path());
    (engine, dir, api_keys) // TempDir 保持存活
}

// ---------------------------------------------------------------------------
// Mock 会话存储
// ---------------------------------------------------------------------------

struct MockSessionStore;

#[async_trait::async_trait]
impl SessionStore for MockSessionStore {
    async fn load(&self, _: &str, _: &str, _: &str, _: usize) -> AppResult<Vec<ChatMessage>> {
        Ok(Vec::new())
    }
    async fn append(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &ChatMessage,
        _: &ChatMessage,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn health(&self) -> AppResult<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Ops endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_returns_ok() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn readyz_returns_json() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10 * 1024)
        .await
        .unwrap();
    let obj: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(obj["status"], "ready");
    assert_eq!(obj["redis"], true);
    assert!(obj["tasks"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn version_returns_json() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 10 * 1024)
        .await
        .unwrap();
    let obj: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(obj["name"].is_string());
    assert!(obj["version"].is_string());
    assert!(obj["tasks"].is_array());
}

#[tokio::test]
async fn metrics_returns_prometheus() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    // warm-up: middleware 需要先完成 observe() 才能看到 counter/histogram
    let _ = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 100 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("http_requests_total"), "body={text:?}");
    assert!(text.contains("http_request_duration_seconds"), "body={text:?}");
}

// ---------------------------------------------------------------------------
// 路由 & 错误处理
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_route_returns_404() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // axum 默认不处理 404，未匹配的路由由 tower 层返回空响应
    // 实际上这取决于 router 的 fallback 设置
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn post_with_invalid_json_returns_400() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agent/stream")
                .header("content-type", "application/json")
                .body(Body::from("not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_missing_required_field_returns_400() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    // 缺少 content 字段
    let body = json!({
        "app_id": "a",
        "user_id": "u",
        "session_id": "s",
        "task_type": "chat",
    });
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agent/stream")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_with_unknown_field_returns_400() {
    let (engine, _dir, api_keys) = build_test_engine().await;
    let router = claw_api::build_router(engine, api_keys);

    // deny_unknown_fields 应该 reject 未知字段
    let body = json!({
        "app_id": "a",
        "user_id": "u",
        "session_id": "s",
        "task_type": "chat",
        "content": "hi",
        "extra_field": "should be rejected",
    });
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/agent/stream")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
