//! POST /v1/agent/stream 的 SSE 处理器。
//!
//! 高并发要点：
//! - Axum 的 `Sse` 响应基于 hyper body stream，天然零缓冲
//! - 心跳 keep-alive 交给 axum 内置，避免中间件断链
//! - 响应 Stream 直接源自 `AgentEngine::run_stream`，无额外复制
//! - 任何错误映射到 SSE `event: error` 后优雅关闭流

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::{stream, Stream, StreamExt};

use claw_agent::{AgentEngine, AgentInput};
use claw_types::{AppError, ConfigHandle};
use claw_llm::LlmDelta;

use crate::dto::AgentRequest;
use crate::error::{ApiError, ApiResult};

pub type SseItem = Result<Event, Infallible>;

/// Axum handler：SSE 流式 Agent 调用。
pub async fn agent_stream(
    State(engine): State<Arc<AgentEngine>>,
    Json(req): Json<AgentRequest>,
) -> ApiResult<Sse<impl Stream<Item = SseItem>>> {
    req.validate().map_err(|e| ApiError(AppError::BadRequest(e.to_string())))?;

    let input = AgentInput {
        app_id: req.app_id,
        user_id: req.user_id,
        session_id: req.session_id,
        task_type: req.task_type,
        content: req.content,
    };

    let request_id = ulid::Ulid::new().to_string();
    let cfg_handle: ConfigHandle = engine.config().clone();
    let cfg = cfg_handle.load();
    let keepalive_secs = cfg.server.sse_keep_alive_secs;

    let upstream = engine.run_stream(input).await?;

    // 头部 meta 事件（request_id），便于客户端关联日志
    let meta_event = Event::default().event("meta").data(format!(
        "{{\"request_id\":\"{}\"}}",
        request_id
    ));

    // 把 LlmDelta 逐个映射为 SSE Event；Done -> event:done + 关闭流
    // 运行在 react 模式下，engine 会把 tool_call/tool_result 用
    // 带 __claw_event 标记的 JSON 字符串包装进 LlmDelta::Text，这里重新路由到专属 SSE event。
    let mapped = async_stream::stream_from_rx(upstream).map(move |d| -> SseItem {
        let ev = match d {
            LlmDelta::Text(t) => match try_unpack_marker(&t) {
                Some((event_name, payload)) => Event::default().event(event_name).data(payload),
                None => Event::default().event("message").data(t),
            },
            LlmDelta::ToolCalls(_) => {
                // ReAct 路径下不会走到这里（已包装为 Text marker）；plain 路径下忽略。
                Event::default().event("message").data("")
            }
            LlmDelta::Error(e) => Event::default().event("error").data(e),
            LlmDelta::Done => Event::default().event("done").data("[DONE]"),
        };
        Ok(ev)
    });

    let prepended = stream::once(async move { Ok::<Event, Infallible>(meta_event) }).chain(mapped);

    Ok(Sse::new(prepended).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(keepalive_secs))
            .text("keep-alive"),
    ))
}

/// 检测是否是 engine 包装的控制事件（tool_call / tool_result）。
///
/// 约定格式为 JSON：`{"__claw_event": "tool_call"|"tool_result", ...payload}`。
/// 返回 (event_name, payload_json_string)，其中 payload 已剔除 marker 字段。
fn try_unpack_marker(s: &str) -> Option<(&'static str, String)> {
    // 快速路径：绝大多数 text chunk 不含标记，先做字符串扫描避免全量 JSON 反序列化
    if !s.starts_with('{') || !s.contains("\"__claw_event\"") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let event_name = v.get("__claw_event")?.as_str()?;
    let mapped = match event_name {
        "tool_call" => "tool_call",
        "tool_result" => "tool_result",
        "thought" => "thought",
        _ => return None,
    };
    // 剔除 marker 后重新序列化
    let mut obj = match v {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };
    obj.remove("__claw_event");
    Some((mapped, serde_json::Value::Object(obj).to_string()))
}

// 辅助：把 `impl Stream<Item = LlmDelta>` 包装为一个「直到 Done 为止」的流。
// 避免客户端断开后仍拉取上游不必要的数据。
mod async_stream {
    use futures::Stream;
    use pin_project_lite::pin_project;

    use claw_llm::LlmDelta;

    pin_project! {
        pub struct UntilDone<S> {
            #[pin]
            inner: S,
            done: bool,
        }
    }

    impl<S> Stream for UntilDone<S>
    where
        S: Stream<Item = LlmDelta>,
    {
        type Item = LlmDelta;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            let this = self.project();
            if *this.done {
                return std::task::Poll::Ready(None);
            }
            match this.inner.poll_next(cx) {
                std::task::Poll::Ready(Some(d)) => {
                    let is_done = matches!(d, LlmDelta::Done | LlmDelta::Error(_));
                    if is_done {
                        *this.done = true;
                    }
                    std::task::Poll::Ready(Some(d))
                }
                o => o,
            }
        }
    }

    pub fn stream_from_rx<S>(s: S) -> UntilDone<S>
    where
        S: Stream<Item = LlmDelta>,
    {
        UntilDone { inner: s, done: false }
    }
}
