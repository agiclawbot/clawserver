use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
    Closed = 0,
    Open = 1,
    HalfOpen = 2,
}

impl State {
    #[inline]
    fn from_u8(v: u8) -> Self {
        match v {
            1 => State::Open,
            2 => State::HalfOpen,
            _ => State::Closed,
        }
    }
}

pub struct CircuitBreaker {
    name: &'static str,
    failure_ratio: f64,
    min_samples: u32,
    rolling_window: Duration,
    open_duration: Duration,
    half_open_probes: u32,

    state_word: AtomicU64,
    window_started_ms: AtomicU64,
    success: AtomicU64,
    failure: AtomicU64,
    half_open_inflight: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[inline]
fn pack(state: State, ts_ms: u64) -> u64 {
    ((state as u64) << 56) | (ts_ms & 0x00FF_FFFF_FFFF_FFFF)
}

#[inline]
fn unpack(word: u64) -> (State, u64) {
    (State::from_u8((word >> 56) as u8), word & 0x00FF_FFFF_FFFF_FFFF)
}

impl CircuitBreaker {
    pub fn new(
        name: &'static str,
        failure_ratio: f64,
        min_samples: u32,
        rolling_window: Duration,
        open_duration: Duration,
        half_open_probes: u32,
    ) -> Self {
        Self {
            name,
            failure_ratio,
            min_samples,
            rolling_window,
            open_duration,
            half_open_probes,
            state_word: AtomicU64::new(pack(State::Closed, now_ms())),
            window_started_ms: AtomicU64::new(now_ms()),
            success: AtomicU64::new(0),
            failure: AtomicU64::new(0),
            half_open_inflight: AtomicU64::new(0),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn try_acquire(&self) -> bool {
        let now = now_ms();
        let word = self.state_word.load(Ordering::Acquire);
        let (state, since) = unpack(word);
        match state {
            State::Closed => true,
            State::Open => {
                if now.saturating_sub(since) >= self.open_duration.as_millis() as u64 {
                    let new_word = pack(State::HalfOpen, now);
                    if self
                        .state_word
                        .compare_exchange(word, new_word, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        self.half_open_inflight.store(0, Ordering::Release);
                    }
                    self.try_half_open_slot()
                } else {
                    false
                }
            }
            State::HalfOpen => self.try_half_open_slot(),
        }
    }

    fn try_half_open_slot(&self) -> bool {
        let cur = self.half_open_inflight.fetch_add(1, Ordering::AcqRel);
        if cur < self.half_open_probes as u64 {
            true
        } else {
            self.half_open_inflight.fetch_sub(1, Ordering::AcqRel);
            false
        }
    }

    pub fn on_success(&self) {
        self.maybe_reset_window();
        self.success.fetch_add(1, Ordering::Relaxed);
        let word = self.state_word.load(Ordering::Acquire);
        let (state, _) = unpack(word);
        if state == State::HalfOpen {
            let ok = self.success.load(Ordering::Relaxed);
            if ok >= self.half_open_probes as u64 {
                let _ = self.state_word.compare_exchange(
                    word,
                    pack(State::Closed, now_ms()),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                self.success.store(0, Ordering::Release);
                self.failure.store(0, Ordering::Release);
            }
        }
    }

    pub fn on_failure(&self) {
        self.maybe_reset_window();
        let f = self.failure.fetch_add(1, Ordering::Relaxed) + 1;
        let s = self.success.load(Ordering::Relaxed);
        let total = f + s;
        let word = self.state_word.load(Ordering::Acquire);
        let (state, _) = unpack(word);
        match state {
            State::Closed => {
                if total >= self.min_samples as u64 {
                    let ratio = f as f64 / total as f64;
                    if ratio >= self.failure_ratio {
                        let _ = self.state_word.compare_exchange(
                            word,
                            pack(State::Open, now_ms()),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                    }
                }
            }
            State::HalfOpen => {
                let _ = self.state_word.compare_exchange(
                    word,
                    pack(State::Open, now_ms()),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                self.success.store(0, Ordering::Release);
                self.failure.store(0, Ordering::Release);
            }
            State::Open => {}
        }
    }

    fn maybe_reset_window(&self) {
        let now = now_ms();
        let start = self.window_started_ms.load(Ordering::Acquire);
        if now.saturating_sub(start) >= self.rolling_window.as_millis() as u64 {
            if self
                .window_started_ms
                .compare_exchange(start, now, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.success.store(0, Ordering::Release);
                self.failure.store(0, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_allows_traffic() {
        let cb = CircuitBreaker::new(
            "test", 0.5, 5, Duration::from_secs(60), Duration::from_secs(5), 2,
        );
        assert!(cb.try_acquire());
    }

    #[test]
    fn trips_open_after_failure_threshold() {
        let cb = CircuitBreaker::new(
            "test", 0.3, 10, Duration::from_secs(60), Duration::from_secs(60), 2,
        );
        for _ in 0..7 {
            cb.on_success();
        }
        for _ in 0..5 {
            cb.on_failure();
        }
        assert!(!cb.try_acquire());
    }

    #[test]
    fn half_open_probe_slot_limited() {
        let cb = CircuitBreaker::new(
            "test", 0.5, 3, Duration::from_secs(60), Duration::from_millis(10), 2,
        );
        cb.on_failure();
        cb.on_failure();
        cb.on_failure();
        assert!(!cb.try_acquire(), "should be open");

        std::thread::sleep(Duration::from_millis(15));
        assert!(cb.try_acquire(), "half-open should allow first probe");
        assert!(cb.try_acquire(), "half-open should allow second probe");
        assert!(!cb.try_acquire(), "third should be denied");
    }

    #[test]
    fn state_enum_round_trip() {
        assert_eq!(State::from_u8(0), State::Closed);
        assert_eq!(State::from_u8(1), State::Open);
        assert_eq!(State::from_u8(2), State::HalfOpen);
        assert_eq!(State::from_u8(99), State::Closed);
    }
}
