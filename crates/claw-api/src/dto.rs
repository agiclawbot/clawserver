//! 请求 / 响应 DTO。
//!
//! 使用 `#[serde(deny_unknown_fields)]` 保证入参严格，避免脏字段被静默忽略。

use serde::{Deserialize, Serialize};

// ---- 字段长度限制 ----
const MAX_APP_ID: usize = 64;
const MAX_USER_ID: usize = 64;
const MAX_SESSION_ID: usize = 128;
const MAX_TASK_TYPE: usize = 64;
const MAX_MODEL: usize = 128;
const MAX_CONTENT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub app_id: String,
    pub user_id: String,
    pub session_id: String,
    pub task_type: String,
    pub content: String,

    /// 可选：覆盖模型（仅允许 provider 内定义的 model 名）。
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

impl AgentRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        Self::check_required(&self.app_id, MAX_APP_ID)?;
        Self::check_required(&self.user_id, MAX_USER_ID)?;
        Self::check_required(&self.session_id, MAX_SESSION_ID)?;
        Self::check_required(&self.task_type, MAX_TASK_TYPE)?;
        Self::check_required(&self.content, MAX_CONTENT_BYTES)?;

        if let Some(ref m) = self.model {
            if m.trim().is_empty() || m.len() > MAX_MODEL {
                return Err("model invalid");
            }
        }
        Ok(())
    }

    fn check_required(v: &str, max: usize) -> Result<(), &'static str> {
        if v.trim().is_empty() {
            return Err("field required");
        }
        if v.len() > max {
            return Err("field too long");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentResponseMeta {
    pub request_id: String,
    pub task_type: String,
}
