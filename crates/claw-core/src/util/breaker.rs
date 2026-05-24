//! 无锁熔断器。
//!
//! 实现思路（参考 Netflix Hystrix，裁剪为纯原子实现）：
//! - 三态：Closed / Open / HalfOpen
//! - 状态位存在单个 `AtomicU64` 中（高 8 位状态，低 56 位状态切换时间戳 ms）
//!   以单次 `compare_exchange` 完成状态迁移，零锁
//! - 滚动窗口成功 / 失败计数分别为独立 `AtomicU64`，溢出自动重置
//! - 当 Closed 下达到失败率阈值 -> 原子切换为 Open
//! - Open 超过 open_duration -> 切换为 HalfOpen，放行 N 个探测
//! - HalfOpen 下探测成功比例达标 -> Closed

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

    // 高 8 位 state，低 56 位 ms 时间戳（切换时刻）
    state_word: AtomicU64,
    // 滚动窗口开始 ms
    window_started_ms: AtomicU64,
    success: AtomicU64,
    failure: AtomicU64,
    // HalfOpen 下已放行的探测数
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

    /// 尝试获取一次调用许可；返回 true 表示可执行下游。
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
