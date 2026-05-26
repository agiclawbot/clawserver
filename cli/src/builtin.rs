//! claw-cli 自带的最小内置工具：
//!
//! - 默认（`tools-builtin` feature 开）注册 `claw_agent::tools::builtin::{TimeNow, HttpGet, WebSearch}`
//! - 始终注册一个简单的 [`Echo`]（cli-only 调试工具，无外部依赖）
//!
//! 设计：仅依赖 [`claw_llm::Tool`] 契约，cli 端可通过关闭 `tools-builtin` feature
//! 退化成最小 Echo 工具集。

use std::sync::Arc;

use async_trait::async_trait;
use claw_types::AppResult;
use claw_llm::{Tool, ToolRegistry};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// echo (cli-only)
// ---------------------------------------------------------------------------
pub struct Echo;

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Return the input string as-is. Useful for verifying tool calling plumbing."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to echo back." }
            },
            "required": ["text"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> AppResult<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                claw_types::AppError::BadRequest(
                    "missing required field `text`".into(),
                )
            })?;
        Ok(text.to_string())
    }
}

// ---------------------------------------------------------------------------
// 装配：调试期默认注册全部内置工具
// ---------------------------------------------------------------------------

/// 构造调试用默认工具注册表。
///
/// - `tools-builtin` 开启时：注册 `claw_agent::tools::builtin::{TimeNow, HttpGet, WebSearch}`
/// - 始终额外注册 cli-only 的 Echo
pub fn build_default_registry() -> Arc<ToolRegistry> {
    let mut reg = ToolRegistry::new();

    #[cfg(feature = "tools-builtin")]
    {
        reg.register(Arc::new(claw_agent::tools::builtin::TimeNow));
        reg.register(Arc::new(claw_agent::tools::builtin::HttpGet::new()));
        reg.register(Arc::new(claw_agent::tools::builtin::WebSearch));
    }

    reg.register(Arc::new(Echo));
    Arc::new(reg)
}

/// cli 自带的已知工具名（与 [`build_default_registry`] 同步维护）。
pub fn known_tool_names() -> &'static [&'static str] {
    #[cfg(feature = "tools-builtin")]
    {
        &["time_now", "http_get", "web_search", "echo"]
    }
    #[cfg(not(feature = "tools-builtin"))]
    {
        &["echo"]
    }
}
