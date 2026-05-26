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
    pub fn assistant_tool_calls(calls: Vec<AssistantToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: String::new(),
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: AssistantFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantFunctionCall {
    pub name: String,
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
