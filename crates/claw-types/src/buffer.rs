use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BufferConfig {
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
