//! LlmPool 构建 / 选路 / 默认 provider 行为。
//!
//! 不发起真实网络调用：只验证 builder + 路由分发。

use std::collections::HashMap;

use claw_llm::config::{CircuitBreakerConfig, LlmConfig, LlmProviderConfig, RetryConfig};
use claw_llm::LlmPool;

fn provider(default_model: &str) -> LlmProviderConfig {
    LlmProviderConfig {
        base_url: "http://127.0.0.1:9999".into(),
        api_key_env: "CLAW_TEST_KEY_DOES_NOT_EXIST".into(),
        pool_idle_per_host: 4,
        pool_max_idle_secs: 30,
        request_timeout_secs: 5,
        connect_timeout_secs: 2,
        default_model: default_model.into(),
    }
}

fn breaker_cfg() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_ratio: 0.5,
        min_samples: 4,
        rolling_window_secs: 60,
        open_duration_secs: 5,
        half_open_max_probes: 2,
    }
}

fn retry_cfg() -> RetryConfig {
    RetryConfig {
        max_attempts: 3,
        base_backoff_ms: 100,
        max_backoff_ms: 2_000,
    }
}

fn dual_provider_cfg() -> LlmConfig {
    let mut providers = HashMap::new();
    providers.insert("openai".into(), provider("gpt-4o-mini"));
    providers.insert("deepseek".into(), provider("deepseek-chat"));
    LlmConfig {
        default_provider: "openai".into(),
        providers,
        retry: retry_cfg(),
    }
}

#[test]
fn build_succeeds_with_multiple_providers() {
    let pool = LlmPool::build(&dual_provider_cfg(), &breaker_cfg(), 256).expect("build pool");
    assert_eq!(pool.default_provider(), "openai");
    assert!(pool.get("openai").is_ok());
    assert!(pool.get("deepseek").is_ok());
}

#[test]
fn get_unknown_provider_returns_llm_error() {
    let pool = LlmPool::build(&dual_provider_cfg(), &breaker_cfg(), 256).unwrap();
    let err = match pool.get("ghost") {
        Ok(_) => panic!("expected error for unknown provider"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(msg.contains("provider"), "msg={msg}");
    assert!(msg.contains("ghost"));
}

#[test]
fn get_dyn_returns_chat_provider_object() {
    let pool = LlmPool::build(&dual_provider_cfg(), &breaker_cfg(), 256).unwrap();
    let dyn_provider = pool.get_dyn("openai").unwrap();
    assert_eq!(dyn_provider.name(), "openai");
}

#[test]
fn build_with_empty_providers_yields_empty_pool() {
    let cfg = LlmConfig {
        default_provider: "none".into(),
        providers: HashMap::new(),
        retry: retry_cfg(),
    };
    let pool = LlmPool::build(&cfg, &breaker_cfg(), 256).unwrap();
    assert_eq!(pool.default_provider(), "none");
    assert!(pool.get("none").is_err());
}
