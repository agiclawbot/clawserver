//! ReAct 循环状态机：把"单次 LLM 调用"扩展为"思考 → 工具调用 → 观察 → 再思考"的多轮循环。
//!
//! 设计要点：
//! - 输入：初始 messages + 工具白名单 + 上限 max_iterations + LLM 客户端 + 工具注册表
//! - 输出：单一 `mpsc::Receiver<ReactEvent>`，对外屏蔽内部多轮细节
//! - 业务事件类型见 `ReactEvent`：Text / ToolCall / ToolResult / Done / Error
//! - 单次循环顺序：
//!     1. 调一次 LLM（带 tools）
//!     2. 若 LLM 输出 tool_calls → 顺序执行所有工具，把结果写回 messages，进入下一轮
//!     3. 若 LLM 输出文本 → 直接 forward 到下游，循环结束
//! - 防死循环：max_iterations 兜底；超过则强制把"请用现有信息总结"的 system 消息塞进去再走一轮
//!
//! 高并发要点：
//! - 整个循环放在一个 spawn 出去的 task 内，对外只暴露 `Receiver`
//! - 工具调用顺序执行（同一轮内），避免 LLM 上下文混乱；如需并行可在工具内部自行 spawn
//! - 所有共享数据通过 `Arc` 持有，无锁

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use claw_core::chat::{AssistantToolCall, ChatMessage};
use claw_core::error::AppResult;
use claw_core::llm::{ChatProvider, LlmDelta, LlmRequest};
use claw_core::tool::{ToolCall, ToolRegistry, ToolResult, ToolSpec};

/// ReAct 循环对外发出的事件。
#[derive(Debug, Clone)]
pub enum ReactEvent {
    /// 文本增量（最终答案的流式吐字）。
    Text(String),
    /// 工具被调用（已解析出 name + args）。
    ToolCall(ToolCall),
    /// 工具执行结果。
    ToolResult(ToolResult),
    /// 循环结束。
    Done,
    /// 错误。
    Error(String),
}

/// ReAct 循环参数。
pub struct ReactConfig {
    pub max_iterations: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub timeout: Duration,
    pub provider: String,
    pub model: String,
}

/// 启动 ReAct 循环，返回事件流。
pub fn run_react(
    llm: Arc<dyn ChatProvider>,
    tools: Arc<ToolRegistry>,
    tool_specs: Vec<ToolSpec>,
    initial_messages: Vec<ChatMessage>,
    cfg: ReactConfig,
    channel_buffer: usize,
) -> mpsc::Receiver<ReactEvent> {
    let (tx, rx) = mpsc::channel::<ReactEvent>(channel_buffer);

    tokio::spawn(async move {
        if let Err(e) = react_loop(llm, tools, tool_specs, initial_messages, cfg, tx.clone()).await
        {
            let _ = tx.send(ReactEvent::Error(e.to_string())).await;
        }
        let _ = tx.send(ReactEvent::Done).await;
    });

    rx
}

async fn react_loop(
    llm: Arc<dyn ChatProvider>,
    tools: Arc<ToolRegistry>,
    tool_specs: Vec<ToolSpec>,
    mut messages: Vec<ChatMessage>,
    cfg: ReactConfig,
    tx: mpsc::Sender<ReactEvent>,
) -> AppResult<()> {
    let max_iter = cfg.max_iterations.max(1);

    for iter in 0..max_iter {
        // 最后一轮强制不允许再继续调工具，要求模型给出最终答案
        let last_round = iter + 1 == max_iter;
        let req_tools = if last_round { Vec::new() } else { tool_specs.clone() };

        let req = LlmRequest {
            provider: cfg.provider.clone(),
            model: cfg.model.clone(),
            messages: messages.clone(),
            temperature: cfg.temperature,
            top_p: cfg.top_p,
            max_tokens: cfg.max_tokens,
            stream: true,
            timeout: cfg.timeout,
            tools: req_tools,
        };

        let mut llm_rx = llm.chat_stream(req).await?;

        let mut buf_text = String::new();
        let mut pending_tool_calls: Option<Vec<ToolCall>> = None;
        let mut had_error = false;

        while let Some(d) = llm_rx.recv().await {
            match d {
                LlmDelta::Text(t) => {
                    buf_text.push_str(&t);
                    if tx.send(ReactEvent::Text(t)).await.is_err() {
                        return Ok(()); // 下游断
                    }
                }
                LlmDelta::ToolCalls(calls) => {
                    pending_tool_calls = Some(calls);
                }
                LlmDelta::Error(e) => {
                    had_error = true;
                    let _ = tx.send(ReactEvent::Error(e)).await;
                }
                LlmDelta::Done => break,
            }
        }

        if had_error {
            return Ok(());
        }

        // 本轮模型决定调工具
        if let Some(calls) = pending_tool_calls {
            // 把 assistant 的 tool_calls 写进 messages（OpenAI 协议要求）
            let assistant_calls: Vec<AssistantToolCall> = calls.iter().map(Into::into).collect();
            messages.push(ChatMessage::assistant_tool_calls(assistant_calls));

            // 顺序执行每个工具调用
            for call in &calls {
                if tx.send(ReactEvent::ToolCall(call.clone())).await.is_err() {
                    return Ok(());
                }
                let result = match tools.invoke(&call.name, call.arguments.clone()).await {
                    Ok(s) => ToolResult::ok(&call.id, &call.name, s),
                    Err(e) => ToolResult::err(&call.id, &call.name, e.to_string()),
                };
                // 工具结果消息（OpenAI 要求 role="tool" + tool_call_id）
                messages.push(ChatMessage::tool(&call.id, result.content.clone()));
                if tx.send(ReactEvent::ToolResult(result)).await.is_err() {
                    return Ok(());
                }
            }
            // 进入下一轮循环
            continue;
        }

        // 本轮模型未调工具：要么是中间思考（罕见），要么是最终答案；统一收尾
        if !buf_text.is_empty() {
            messages.push(ChatMessage::assistant(buf_text));
        }
        return Ok(());
    }

    // 走到这里说明用尽 max_iterations 也没出最终答案
    let _ = tx
        .send(ReactEvent::Error(format!(
            "max_iterations({max_iter}) reached without a final answer"
        )))
        .await;
    Ok(())
}
