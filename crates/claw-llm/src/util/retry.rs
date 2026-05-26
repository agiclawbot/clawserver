use std::time::Duration;

use rand::Rng;

#[inline]
pub fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_zero_attempt_is_around_base() {
        let d = backoff(0, 100, 5000);
        assert!(d >= Duration::from_millis(75));
        assert!(d <= Duration::from_millis(125));
    }

    #[test]
    fn backoff_is_bounded_and_monotonic_in_expectation() {
        let d1 = backoff(0, 100, 5000);
        let d2 = backoff(1, 100, 5000);
        let d3 = backoff(5, 100, 5000);
        assert!(d2 >= d1);
        assert!(d3 >= d2);
        assert!(d3 <= Duration::from_millis(5000));
    }
}
