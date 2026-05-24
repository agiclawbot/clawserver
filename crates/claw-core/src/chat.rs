//! Chat 消息模型 —— 与 OpenAI Chat Completions 协议对齐。
//!
//! 设计：
//! - 仅纯数据 + serde，无任何 HTTP / runtime 依赖
//! - 兼容所有角色：System / User / Assistant（可携带 tool_calls）/ Tool（可携带 tool_call_id）
//! - 与 [`crate::tool::ToolCall`] 互转，供 ReAct 循环组装多轮上下文

use serde::Serialize;

use crate::tool::ToolCall;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// 传输给 LLM 的单条消息。
///
/// 兼容 OpenAI Chat Completions 协议所有角色：
/// - System / User：仅 `role` + `content`
/// - Assistant：可携带 `tool_calls`（表示本轮调用了工具）
/// - Tool：可携带 `tool_call_id`（指向上一轮 assistant 发出的某个 tool_call）
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AssistantToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    /// assistant 发出 tool_calls 的中间记录。
    pub fn assistant_tool_calls(calls: Vec<AssistantToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: String::new(),
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }
    /// tool 角色响应某一个 tool_call_id 的执行结果。
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// OpenAI assistant 消息里的 tool_calls 项（发送给 LLM 用）。
#[derive(Debug, Clone, Serialize)]
pub struct AssistantToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str, // 固定 "function"
    pub function: AssistantFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantFunctionCall {
    pub name: String,
    /// OpenAI 定义为 JSON 字符串（不是 object）。
    pub arguments: String,
}

impl From<&ToolCall> for AssistantToolCall {
    fn from(c: &ToolCall) -> Self {
        AssistantToolCall {
            id: c.id.clone(),
            kind: "function",
            function: AssistantFunctionCall {
                name: c.name.clone(),
                arguments: c.arguments.to_string(),
            },
        }
    }
}
