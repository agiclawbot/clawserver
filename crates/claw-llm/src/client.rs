//! OpenAI 兼容的异步 LLM 客户端与全局调用池。
//!
//! 高并发要点：
//! - `reqwest::Client` 自身携带异步连接池，跨线程共享 (`Clone` 廉价)
//! - 流式响应以 `bytes_stream` 增量拉取，按 SSE 行切分，逐块转发，零拷贝
//! - 每 provider 绑定一个 `CircuitBreaker`，热点路径无锁决策
//! - 失败按指数退避重试（仅非 200 且非业务错误的瞬时故障）

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use claw_core::chat::ChatMessage;
use claw_core::error::{AppError, AppResult};
use claw_core::llm::{ChatProvider, LlmDelta, LlmRequest};
use claw_core::tool::{ToolCall, ToolSpec};
use claw_core::util::breaker::CircuitBreaker;
use claw_core::util::retry::backoff;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::config::{CircuitBreakerConfig, LlmConfig, LlmProviderConfig, RetryConfig};

// ===================== 客户端 =====================

pub struct LlmClient {
    name: String,
    cfg: LlmProviderConfig,
    api_key: String,
    http: reqwest::Client,
    breaker: Arc<CircuitBreaker>,
    retry: RetryConfig,
    channel_buffer: usize,
}

impl LlmClient {
    pub fn new(
        name: String,
        cfg: LlmProviderConfig,
        retry: RetryConfig,
        breaker: Arc<CircuitBreaker>,
        channel_buffer: usize,
    ) -> AppResult<Self> {
        // 允许 API Key 为空（例如 ollama 本地），不强制
        let api_key = std::env::var(&cfg.api_key_env).unwrap_or_default();
        let http = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(cfg.pool_max_idle_secs))
            .pool_max_idle_per_host(cfg.pool_idle_per_host)
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(cfg.connect_timeout_secs))
            .timeout(Duration::from_secs(cfg.request_timeout_secs))
            .https_only(false)
            .build()
            .map_err(|e| AppError::Llm(format!("build http client: {e}")))?;
        Ok(Self {
            name,
            cfg,
            api_key,
            http,
            breaker,
            retry,
            channel_buffer,
        })
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn default_model(&self) -> &str {
        &self.cfg.default_model
    }

    /// 流式 chat：返回异步 channel；读取端 drop 后内部会自动中止。
    pub async fn chat_stream(&self, req: LlmRequest) -> AppResult<mpsc::Receiver<LlmDelta>> {
        if !self.breaker.try_acquire() {
            return Err(AppError::CircuitOpen("llm"));
        }
        let url = format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/'));

        // 构造 payload（零拷贝 Serialize）
        #[derive(Serialize)]
        struct Payload<'a> {
            model: &'a str,
            messages: &'a [ChatMessage],
            temperature: f32,
            top_p: f32,
            max_tokens: u32,
            stream: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            tools: Option<&'a [ToolSpec]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            tool_choice: Option<&'static str>,
        }
        let tools_opt = if req.tools.is_empty() {
            None
        } else {
            Some(req.tools.as_slice())
        };
        let payload = Payload {
            model: &req.model,
            messages: &req.messages,
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            stream: true,
            tools: tools_opt,
            tool_choice: tools_opt.map(|_| "auto"),
        };
        let body = Bytes::from(serde_json::to_vec(&payload)?);

        // 重试（仅瞬时故障）
        let mut last_err: Option<AppError> = None;
        let mut resp_opt = None;
        for attempt in 0..self.retry.max_attempts {
            let mut builder = self
                .http
                .post(&url)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .timeout(req.timeout)
                .body(body.clone());
            if !self.api_key.is_empty() {
                builder = builder.bearer_auth(&self.api_key);
            }
            match builder.send().await {
                Ok(r) if r.status().is_success() => {
                    resp_opt = Some(r);
                    break;
                }
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    last_err = Some(AppError::Llm(format!("status {status}: {text}")));
                    if !status.is_server_error() && status.as_u16() != 429 {
                        break;
                    }
                }
                Err(e) => {
                    last_err = Some(AppError::Llm(e.to_string()));
                }
            }
            if attempt + 1 < self.retry.max_attempts {
                tokio::time::sleep(backoff(
                    attempt,
                    self.retry.base_backoff_ms,
                    self.retry.max_backoff_ms,
                ))
                .await;
            }
        }

        let resp = match resp_opt {
            Some(r) => r,
            None => {
                self.breaker.on_failure();
                return Err(last_err
                    .unwrap_or_else(|| AppError::Llm("llm: unknown error".into())));
            }
        };

        // 将 bytes_stream 转为逐行 SSE -> LlmDelta 的 channel
        // 容量由 buffer.channel_size 配置控制
        let (tx, rx) = mpsc::channel::<LlmDelta>(self.channel_buffer);
        let breaker = Arc::clone(&self.breaker);
        tokio::spawn(async move {
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::with_capacity(1024);
            let mut tc_acc = ToolCallAccum::default();
            let mut had_error = false;
            while let Some(chunk_res) = byte_stream.next().await {
                match chunk_res {
                    Ok(chunk) => {
                        if !process_chunk(&chunk, &mut buf, &tx, &mut tc_acc).await {
                            return;
                        }
                    }
                    Err(e) => {
                        had_error = true;
                        let _ = tx.send(LlmDelta::Error(e.to_string())).await;
                        break;
                    }
                }
            }
            if !buf.is_empty() {
                let _ = emit_line(&buf, &tx, &mut tc_acc).await;
                buf.clear();
            }
            if let Some(calls) = tc_acc.take_if_any() {
                let _ = tx.send(LlmDelta::ToolCalls(calls)).await;
            }
            if had_error {
                breaker.on_failure();
            } else {
                breaker.on_success();
            }
            let _ = tx.send(LlmDelta::Done).await;
        });

        Ok(rx)
    }
}

/// 多个 SSE 帧中的 tool_calls 累积器：OpenAI 流式组装是按 index 并多帧叠加函数名/参数的。
#[derive(Default)]
struct ToolCallAccum {
    finished: bool,
    items: Vec<ToolCallBuilder>,
}

struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccum {
    fn ensure_index(&mut self, index: usize) {
        while self.items.len() <= index {
            self.items.push(ToolCallBuilder {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        }
    }
    fn merge(&mut self, item: &StreamToolCall) {
        let idx = item.index.unwrap_or(0) as usize;
        self.ensure_index(idx);
        let b = &mut self.items[idx];
        if let Some(id) = &item.id {
            if !id.is_empty() {
                b.id = id.clone();
            }
        }
        if let Some(f) = &item.function {
            if let Some(n) = &f.name {
                if !n.is_empty() {
                    b.name = n.clone();
                }
            }
            if let Some(a) = &f.arguments {
                if !a.is_empty() {
                    b.arguments.push_str(a);
                }
            }
        }
    }
    fn take_if_any(&mut self) -> Option<Vec<ToolCall>> {
        if self.items.is_empty() {
            return None;
        }
        let drained = std::mem::take(&mut self.items);
        let calls: Vec<ToolCall> = drained
            .into_iter()
            .filter(|b| !b.name.is_empty())
            .map(|b| {
                let args: Value = if b.arguments.trim().is_empty() {
                    Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&b.arguments)
                        .unwrap_or_else(|_| Value::String(b.arguments.clone()))
                };
                let id = if b.id.is_empty() {
                    format!("call_{}", ulid::Ulid::new())
                } else {
                    b.id.clone()
                };
                ToolCall {
                    id,
                    name: b.name.clone(),
                    arguments: args,
                }
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    }
}

async fn process_chunk(
    chunk: &Bytes,
    buf: &mut Vec<u8>,
    tx: &mpsc::Sender<LlmDelta>,
    tc_acc: &mut ToolCallAccum,
) -> bool {
    buf.extend_from_slice(chunk);
    loop {
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let trimmed = trim_line(&line);
            if trimmed.is_empty() {
                continue;
            }
            if !emit_line(trimmed, tx, tc_acc).await {
                return false;
            }
        } else {
            break;
        }
    }
    true
}

fn trim_line(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    &line[..end]
}

async fn emit_line(
    line: &[u8],
    tx: &mpsc::Sender<LlmDelta>,
    tc_acc: &mut ToolCallAccum,
) -> bool {
    let prefix = b"data:";
    if !line.starts_with(prefix) {
        return true;
    }
    let payload = &line[prefix.len()..];
    let payload = match payload.first() {
        Some(b' ') => &payload[1..],
        _ => payload,
    };
    if payload == b"[DONE]" {
        return true;
    }
    match serde_json::from_slice::<StreamChunk>(payload) {
        Ok(c) => {
            for ch in c.choices {
                if let Some(delta) = ch.delta {
                    if let Some(txt) = delta.content {
                        if !txt.is_empty() {
                            if tx.send(LlmDelta::Text(txt)).await.is_err() {
                                return false;
                            }
                        }
                    }
                    if let Some(tcs) = delta.tool_calls {
                        for it in &tcs {
                            tc_acc.merge(it);
                        }
                    }
                }
                if let Some(reason) = ch.finish_reason {
                    if reason == "tool_calls" {
                        if let Some(calls) = tc_acc.take_if_any() {
                            tc_acc.finished = true;
                            if tx.send(LlmDelta::ToolCalls(calls)).await.is_err() {
                                return false;
                            }
                        }
                    }
                }
            }
            true
        }
        Err(_) => true,
    }
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}
#[derive(Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}
#[derive(Deserialize)]
struct StreamToolCall {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionCall>,
}
#[derive(Deserialize)]
struct StreamFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// ===================== 全局 Pool =====================

pub struct LlmPool {
    clients: HashMap<String, Arc<LlmClient>>,
    default_provider: String,
}

impl LlmPool {
    /// 装配池：传入 `LlmConfig`（providers + retry）、`CircuitBreakerConfig`（共享熔断阈值）和 `channel_buffer`（mpsc 容量）。
    pub fn build(
        llm: &LlmConfig,
        breaker_cfg: &CircuitBreakerConfig,
        channel_buffer: usize,
    ) -> AppResult<Arc<Self>> {
        let mut clients = HashMap::with_capacity(llm.providers.len());
        for (name, pcfg) in &llm.providers {
            let breaker = Arc::new(CircuitBreaker::new(
                Box::leak(format!("llm.{name}").into_boxed_str()),
                breaker_cfg.failure_ratio,
                breaker_cfg.min_samples,
                Duration::from_secs(breaker_cfg.rolling_window_secs),
                Duration::from_secs(breaker_cfg.open_duration_secs),
                breaker_cfg.half_open_max_probes,
            ));
            let client = LlmClient::new(
                name.clone(), pcfg.clone(), llm.retry.clone(), breaker, channel_buffer,
            )?;
            clients.insert(name.clone(), Arc::new(client));
        }
        Ok(Arc::new(Self {
            clients,
            default_provider: llm.default_provider.clone(),
        }))
    }

    #[inline]
    pub fn get(&self, provider: &str) -> AppResult<Arc<LlmClient>> {
        self.clients
            .get(provider)
            .cloned()
            .ok_or_else(|| AppError::Llm(format!("provider `{provider}` not found")))
    }

    #[inline]
    pub fn default_provider(&self) -> &str {
        &self.default_provider
    }

    /// 返回抽象 provider 封装，便于未来插入 rig / adk-rust 实现。
    #[inline]
    pub fn get_dyn(&self, provider: &str) -> AppResult<Arc<dyn ChatProvider>> {
        self.clients
            .get(provider)
            .map(|c| c.clone() as Arc<dyn ChatProvider>)
            .ok_or_else(|| AppError::Llm(format!("provider `{provider}` not found")))
    }
}

// 让 LlmClient 自动满足 ChatProvider，供扩展口直接返回 trait 对象。
#[async_trait::async_trait]
impl ChatProvider for LlmClient {
    #[inline]
    fn name(&self) -> &str {
        LlmClient::name(self)
    }

    async fn chat_stream(&self, req: LlmRequest) -> AppResult<mpsc::Receiver<LlmDelta>> {
        LlmClient::chat_stream(self, req).await
    }
}
