//! 内部缓冲配置 —— 控制 mpsc channel 容量等异步缓冲参数。
//!
//! 所有值均有合理默认值，可通过 YAML 配置覆盖，无需修改代码。

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BufferConfig {
    /// 所有内部 mpsc channel 的缓冲容量。
    /// 影响：LLM 流解析 → engine → SSE 的整条链路的背压行为。
    /// 调大：减少 producer 等待 consumer 的概率，降低 tail 延迟，但增加内存占用。
    /// 调小：降低内存占用，提高 backpressure 响应速度。
    #[serde(default = "default_channel_size")]
    pub channel_size: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            channel_size: default_channel_size(),
        }
    }
}

fn default_channel_size() -> usize {
    256
}
