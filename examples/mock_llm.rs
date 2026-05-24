//! 本地 mock LLM 服务（OpenAI Chat Completions 兼容）。
//!
//! 用途：开发期 e2e 验证 SSE 流式链路，不依赖真实 OpenAI / DeepSeek。
//! 运行： `cargo run --example mock_llm`
//! 路由：  POST http://127.0.0.1:9090/v1/chat/completions
//!
//! 行为：
//! - 解析最后一条 user 消息作为输入
//! - 把回答模板按字切分，每 30ms 推一个 chunk，模拟真实 LLM 吐字延迟
//! - 输出标准 OpenAI SSE 帧格式：`data: {...}\n\n` + 终止 `data: [DONE]\n\n`

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::Json,
    response::sse::{Event, KeepAlive, Sse},
    routing::post,
    Router,
};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
struct ChatReq {
    #[allow(dead_code)]
    model: String,
    messages: Vec<Msg>,
    #[serde(default)]
    #[allow(dead_code)]
    stream: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct Msg {
    role: String,
    content: String,
}

async fn chat_completions(
    Json(req): Json<ChatReq>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // 取最后一条 user 消息
    let user_input = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // 构造一个有"内容感"的回复
    let reply = format!(
        "你好！我已收到你的请求。原文是「{}」。这是来自 ClawServer mock LLM 的流式应答，按字逐步返回。",
        user_input.chars().take(50).collect::<String>()
    );

    // 按字切分 → 30ms 间隔
    let chars: Vec<String> = reply.chars().map(|c| c.to_string()).collect();
    let total = chars.len();

    let body = stream::iter(chars.into_iter().enumerate())
        .then(move |(i, ch)| async move {
            // 模拟 token 生成延迟
            tokio::time::sleep(Duration::from_millis(30)).await;
            // 标准 OpenAI 流式 chunk 格式
            let payload = json!({
                "id": "mock-cmpl-1",
                "object": "chat.completion.chunk",
                "created": 0,
                "model": "mock-llm",
                "choices": [{
                    "index": 0,
                    "delta": { "content": ch },
                    "finish_reason": if i + 1 == total { Some("stop") } else { None }
                }]
            });
            Ok::<_, Infallible>(Event::default().data(payload.to_string()))
        })
        .chain(stream::once(async {
            // OpenAI 协议终止帧
            Ok(Event::default().data("[DONE]"))
        }));

    Sse::new(body).keep_alive(KeepAlive::default())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let app = Router::new().route("/v1/chat/completions", post(chat_completions));
    let addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("[mock-llm] listening on http://{addr}/v1/chat/completions");
    axum::serve(listener, app).await.unwrap();
}
