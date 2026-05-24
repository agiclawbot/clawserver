//! 指数退避重试（带抖动）。
//!
//! 纯函数式，不持状态；适合在热点路径上逐次调用计算下次等待时长。

use std::time::Duration;

use rand::Rng;

#[inline]
pub fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    // 2^attempt，含 25% 抖动；attempt 从 0 开始
    let exp = 1u64.checked_shl(attempt.min(20)).unwrap_or(u64::MAX);
    let raw = base_ms.saturating_mul(exp).min(max_ms);
    let jitter_high = (raw as f64 * 0.25) as u64;
    let jitter = if jitter_high == 0 {
        0
    } else {
        rand::thread_rng().gen_range(0..=jitter_high)
    };
    Duration::from_millis(raw.saturating_sub(jitter))
}
