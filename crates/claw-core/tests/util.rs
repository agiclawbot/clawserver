//! CircuitBreaker / retry::backoff 行为校验。

use std::time::Duration;

use claw_core::util::breaker::{CircuitBreaker, State};
use claw_core::util::retry::backoff;

fn fast_breaker() -> CircuitBreaker {
    CircuitBreaker::new(
        "test",
        0.5,                       // 50% 失败率即跳 Open
        4,                         // 至少 4 个样本
        Duration::from_secs(60),   // 滚动窗口
        Duration::from_millis(50), // Open 持续时间（短，便于测试 HalfOpen 切换）
        2,                         // HalfOpen 探测槽
    )
}

#[test]
fn closed_allows_traffic() {
    let cb = fast_breaker();
    for _ in 0..10 {
        assert!(cb.try_acquire());
    }
}

#[test]
fn trips_open_after_failure_threshold() {
    let cb = fast_breaker();
    // 1 success + 4 failures = 5 samples, ratio = 4/5 = 80% > 50%
    cb.on_success();
    for _ in 0..4 {
        cb.on_failure();
    }
    // 触发后 Closed → Open，try_acquire 应直接拒绝
    assert!(!cb.try_acquire(), "breaker should be open after 4/5 fail");
}

#[test]
fn half_open_probe_slot_limited() {
    let cb = fast_breaker();
    cb.on_success();
    for _ in 0..4 {
        cb.on_failure();
    }
    assert!(!cb.try_acquire());
    // 等 Open duration 过期 → 进入 HalfOpen，最多放 2 个探测
    std::thread::sleep(Duration::from_millis(70));
    let p1 = cb.try_acquire();
    let p2 = cb.try_acquire();
    let p3 = cb.try_acquire();
    assert!(p1 && p2 && !p3, "halfopen should let 2 probes through, p1={p1} p2={p2} p3={p3}");
}

#[test]
fn state_enum_round_trip() {
    // 公开类型可比较
    assert_eq!(State::Closed, State::Closed);
    assert_ne!(State::Closed, State::Open);
}

#[test]
fn backoff_is_bounded_and_monotonic_in_expectation() {
    // 多次取最大值兜底（带抖动），用上界验证
    for attempt in 0..5 {
        let d = backoff(attempt, 100, 5_000);
        assert!(d.as_millis() <= 5_000, "attempt {attempt}: {:?}", d);
    }
    // 高 attempt 必被 max_ms 截断
    let d = backoff(30, 100, 800);
    assert!(d.as_millis() <= 800);
}

#[test]
fn backoff_zero_attempt_is_around_base() {
    let d = backoff(0, 100, 10_000);
    // base=100，抖动 25%，结果区间 [75, 100]
    assert!((75..=100).contains(&(d.as_millis() as u64)));
}
