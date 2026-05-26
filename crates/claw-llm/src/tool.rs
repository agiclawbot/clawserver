use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use claw_types::{AppError, AppResult};

// ========================= 数据模型 =========================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

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

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

// ========================= Tool trait =========================

#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn invoke(&self, args: Value) -> AppResult<String>;

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

// ========================= 注册表 =========================

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

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

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
