//! 工具系统契约：Agent 可调用的原子能力（trait + 数据模型 + 注册表）。
//!
//! 设计要点：
//! - [`Tool`] trait 是对象安全的（dyn 兼容），便于 [`ToolRegistry`] 持有 `Arc<dyn Tool>`
//! - 工具的参数 / 返回值统一用 `serde_json::Value`，匹配 OpenAI tool calling JSON 协议
//! - 工具自描述：name / description / parameters_schema 直接映射到 OpenAI tools 字段
//! - 全异步、`Send + Sync + 'static`，可跨 tokio task 共享
//! - **零业务依赖**：仅 serde / async-trait / tracing；具体工具实现（http_get / web_search 等）
//!   留在上层 crate

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 工具调用请求（LLM 输出的 tool_call 项）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// 工具执行结果。
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(call_id: impl Into<String>, name: impl Into<String>, content: String) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            content,
            is_error: false,
        }
    }
    pub fn err(call_id: impl Into<String>, name: impl Into<String>, msg: String) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            content: msg,
            is_error: true,
        }
    }
}

/// 工具的 OpenAI 风格自描述，序列化后即为请求体里的 `tools[]` 项。
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub kind: &'static str, // 固定 "function"
    pub function: FunctionSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema
}

// ---------------------------------------------------------------------------
// Tool trait
// ---------------------------------------------------------------------------

/// 工具 trait：所有可调用的原子能力都要实现。
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// 工具名称（LLM 用此标识调用）。
    fn name(&self) -> &str;

    /// 给 LLM 看的功能描述，影响选择是否调用本工具。
    fn description(&self) -> &str;

    /// 参数的 JSON Schema（OpenAI 风格）。
    fn parameters_schema(&self) -> Value;

    /// 异步执行；参数为 LLM 输出的 JSON。
    async fn invoke(&self, args: Value) -> AppResult<String>;

    /// 转为 OpenAI tools[] 项。
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            kind: "function",
            function: FunctionSpec {
                name: self.name().to_string(),
                description: self.description().to_string(),
                parameters: self.parameters_schema(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// 注册表：启动时一次性注册，运行期只读
// ---------------------------------------------------------------------------

/// 全局工具注册表：`name -> Arc<dyn Tool>`。
///
/// - 启动时一次性注册，运行期只读 → 用 `HashMap` 即可，无需 RwLock
/// - 通过 `Arc<dyn Tool>` 共享，避免重复构造工具实例
/// - 提供按名查找、按白名单批量取 spec、按白名单批量调用
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册一个工具实例。
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// 按名取工具。
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// 按白名单批量取 OpenAI spec。
    /// - whitelist 为空时返回空（mode=react 但未配工具的情况）。
    /// - whitelist 命名不存在的工具被忽略（仅日志告警），便于配置宽松运行。
    pub fn specs_for(&self, whitelist: &[String]) -> Vec<ToolSpec> {
        whitelist
            .iter()
            .filter_map(|n| match self.tools.get(n) {
                Some(t) => Some(t.spec()),
                None => {
                    tracing::warn!(tool = %n, "tool not found in registry, skipped");
                    None
                }
            })
            .collect()
    }

    /// 调用一个工具（仅当工具不存在或参数错误时返回 Err；工具内部错误用 ToolResult.is_error 表达）。
    pub async fn invoke(&self, name: &str, args: Value) -> AppResult<String> {
        let tool = self
            .get(name)
            .ok_or_else(|| AppError::Internal(format!("tool not registered: {name}")))?;
        tool.invoke(args).await
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
