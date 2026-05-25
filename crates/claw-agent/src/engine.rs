//! # Agent 引擎：业务编排核心
//!
//! ## 职责
//!
//! 位于架构的**编排层**，负责组合所有基础组件来完成一次"Agent 调用"：
//!
//! ```text
//! POST /v1/agent/stream  →  AgentEngine::run_stream()
//!                                   │
//!                     ┌─────────────┼─────────────┐
//!                     ▼             ▼             ▼
//!               TaskRegistry  SessionMemory   LlmPool
//!               (查 task cfg)  (加载历史消息)  (获取 ChatProvider)
//!                     │             │             │
//!                     └─────────────┼─────────────┘
//!                                   ▼
//!                          ReAct 循环 / Plain 流
//!                                   │
//!                                   ▼
//!                          LlmDelta 流 → SSE 响应
//!                                   │
//!                         后台写回 Redis(不阻塞)
//! ```
//!
//! ## 两种运行模式
//!
//! | mode | 行为 | 适用场景 |
//! |------|------|----------|
//! | `plain` | 单次 LLM 流式调用，零额外开销 | 聊天、翻译、总结 |
//! | `react` | 多轮 Thought→Tool→Observation 循环 | 需要调用工具的任务 |
//!
//! ## 无锁设计
//!
//! 所有共享数据为 `Arc<...>`——TaskRegistry / ToolRegistry / LlmPool / SkillRegistry
//! 均是启动期一次构建，运行期只读。不构造任何 Mutex / RwLock。

use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use pin_project_lite::pin_project;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use claw_config::{ConfigHandle, TaskConfig, TaskMode};
use claw_core::chat::ChatMessage;
use claw_core::error::{AppError, AppResult};
use claw_core::llm::{LlmDelta, LlmRequest};
use claw_core::tool::ToolRegistry;
use claw_llm::LlmPool;
use claw_core::skill::{Skill, SkillRegistry};

use crate::memory::SessionStore;
use crate::react::{run_react, ReactConfig, ReactEvent};
use crate::task::TaskRegistry;

#[derive(Debug, Clone)]
pub struct AgentInput {
    pub app_id: String,
    pub user_id: String,
    pub session_id: String,
    pub task_type: String,
    pub content: String,
}

/// ClawServer 核心编排引擎。
///
/// 持有全部运行期依赖，通过 `run_stream()` 对外提供 Agent 调用入口。
///
/// # 字段说明
/// | 字段 | 类型 | 来源 | 作用 |
/// |------|------|------|------|
/// | `cfg` | `ConfigHandle` | 启动时加载 | 运行期只读配置快照 |
/// | `tasks` | `Arc<TaskRegistry>` | `TaskRegistry::build()` | 任务 YAML 的只读索引 |
/// | `memory` | `Arc<dyn SessionStore>` | Redis 或 Mock | 会话历史读写 |
/// | `llm` | `Arc<LlmPool>` | `LlmPool::build()` | 按 provider 获取 LLM 客户端 |
/// | `tools` | `Arc<ToolRegistry>` | `build_tool_registry()` | 所有已注册工具 |
/// | `skills` | `Arc<SkillRegistry>` | 从文件加载 | Skill 指令 + 工具白名单 |
/// | `channel_buffer` | `usize` | `buffer.channel_size` | 内部 mpsc 缓冲大小 |
pub struct AgentEngine {
    cfg: ConfigHandle,
    tasks: Arc<TaskRegistry>,
    memory: Arc<dyn SessionStore>,
    llm: Arc<LlmPool>,
    tools: Arc<ToolRegistry>,
    skills: Arc<SkillRegistry>,
    channel_buffer: usize,
}

impl AgentEngine {
    pub fn new(
        cfg: ConfigHandle,
        tasks: Arc<TaskRegistry>,
        memory: Arc<dyn SessionStore>,
        llm: Arc<LlmPool>,
        tools: Arc<ToolRegistry>,
        skills: Arc<SkillRegistry>,
    ) -> Arc<Self> {
        let channel_buffer = cfg.load().buffer.channel_size;
        Arc::new(Self {
            cfg,
            tasks,
            memory,
            llm,
            tools,
            skills,
            channel_buffer,
        })
    }

    /// 执行一次流式 Agent 调用，返回 SSE 文本流。
    pub async fn run_stream(
        self: &Arc<Self>,
        input: AgentInput,
    ) -> AppResult<impl Stream<Item = LlmDelta> + Send + 'static> {
        let task = self
            .tasks
            .get(&input.task_type)
            .ok_or_else(|| AppError::TaskNotFound(input.task_type.clone()))?;

        // 可选 skill：拼接指令 + 作为工具白名单备选
        let skill = task
            .skill
            .as_deref()
            .and_then(|name| self.skills.get(name));

        // 1) 组装消息（系统 + skill.instruction + 历史记忆 + 本轮 user）
        // 根据 max_turns 预估容量以减少运行时 reallocation
        let estimated_capacity = 2 + task.memory.max_turns * 2; // system + user + history
        let mut messages: Vec<ChatMessage> = Vec::with_capacity(estimated_capacity);
        let combined_system = combine_system(
            &task.prompt.system,
            skill.as_deref().map(|s| s.instruction.as_str()),
        );
        if !combined_system.is_empty() {
            messages.push(ChatMessage::system(combined_system));
        }
        if task.memory.enabled && task.memory.max_turns > 0 {
            match self
                .memory
                .load(
                    &input.app_id,
                    &input.user_id,
                    &input.session_id,
                    task.memory.max_turns,
                )
                .await
            {
                Ok(hist) => messages.extend(hist),
                Err(e) => {
                    tracing::warn!(err = %e, "load memory failed, continue without history");
                }
            }
        }
        let user_content = render_template(&task.prompt.user_template, &input.content);
        let user_msg = ChatMessage::user(user_content);
        messages.push(user_msg.clone());

        // 2) 按 mode 分流
        match task.mode {
            TaskMode::Plain => self.run_plain(&task, messages, user_msg, input).await,
            TaskMode::React => {
                self.run_react_mode(&task, messages, user_msg, input, skill.as_deref())
                    .await
            }
        }
    }

    /// plain 模式：单次 LLM 流式 + 主/fallback 链。
    async fn run_plain(
        self: &Arc<Self>,
        task: &TaskConfig,
        messages: Vec<ChatMessage>,
        user_msg: ChatMessage,
        input: AgentInput,
    ) -> AppResult<DeltaStream<ReceiverStream<LlmDelta>>> {
        // 构造“主 + fallback”尝试链
        let mut chain: Vec<(String, String)> =
            Vec::with_capacity(1 + task.llm.fallback.len());
        chain.push((task.llm.provider.clone(), task.llm.model.clone()));
        for ep in &task.llm.fallback {
            chain.push((ep.provider.clone(), ep.model.clone()));
        }

        let mut last_err: Option<AppError> = None;
        let mut rx_opt: Option<tokio::sync::mpsc::Receiver<LlmDelta>> = None;
        let mut used: Option<(String, String)> = None;
        for (provider, model) in &chain {
            let client = match self.llm.get(provider) {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            let req = LlmRequest {
                provider: provider.clone(),
                model: model.clone(),
                messages: messages.clone(),
                temperature: task.llm.temperature,
                top_p: task.llm.top_p,
                max_tokens: task.llm.max_tokens,
                stream: true,
                timeout: Duration::from_secs(task.timeout_secs),
                tools: Vec::new(),
            };
            match client.chat_stream(req).await {
                Ok(rx) => {
                    rx_opt = Some(rx);
                    used = Some((provider.clone(), model.clone()));
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %provider, model = %model, err = %e,
                        "llm endpoint failed, trying next fallback"
                    );
                    last_err = Some(e);
                }
            }
        }
        let rx = match rx_opt {
            Some(r) => r,
            None => {
                return Err(last_err
                    .unwrap_or_else(|| AppError::Llm("all fallback endpoints failed".into())));
            }
        };
        if let Some((p, m)) = &used {
            tracing::info!(provider = %p, model = %m, "llm endpoint selected (plain)");
        }

        // 边转发边聚合：记忆写回
        let (fwd_tx, fwd_rx) = mpsc::channel::<LlmDelta>(self.channel_buffer);
        let memory = Arc::clone(&self.memory);
        let persist = task.memory.enabled && task.memory.max_turns > 0;
        let app_id = input.app_id.clone();
        let user_id = input.user_id.clone();
        let session_id = input.session_id.clone();
        let save_user = user_msg.clone();
        tokio::spawn(async move {
            let mut rx = rx;
            let mut buf = String::new();
            while let Some(delta) = rx.recv().await {
                match &delta {
                    LlmDelta::Text(t) => buf.push_str(t),
                    LlmDelta::ToolCalls(_) | LlmDelta::Done | LlmDelta::Error(_) => {}
                }
                if fwd_tx.send(delta.clone()).await.is_err() {
                    break;
                }
                if matches!(delta, LlmDelta::Done | LlmDelta::Error(_)) {
                    break;
                }
            }
            drop(fwd_tx);
            if persist && !buf.is_empty() {
                let assistant = ChatMessage::assistant(buf);
                if let Err(e) = memory
                    .append(&app_id, &user_id, &session_id, &save_user, &assistant)
                    .await
                {
                    tracing::warn!(err = %e, "memory append failed");
                }
            }
        });

        Ok(DeltaStream::new(ReceiverStream::new(fwd_rx)))
    }

    /// react 模式：ReAct 多轮循环，输出转换为 LlmDelta。
    async fn run_react_mode(
        self: &Arc<Self>,
        task: &TaskConfig,
        messages: Vec<ChatMessage>,
        user_msg: ChatMessage,
        input: AgentInput,
        skill: Option<&Skill>,
    ) -> AppResult<DeltaStream<ReceiverStream<LlmDelta>>> {
        // 工具白名单：task.tools 优先，其次 skill.tools
        let whitelist: Vec<String> = if !task.tools.is_empty() {
            task.tools.clone()
        } else if let Some(s) = skill {
            s.manifest.tools.clone()
        } else {
            Vec::new()
        };
        let tool_specs = self.tools.specs_for(&whitelist);

        // 只走主 provider（react 不走 fallback 链，避免轮中切换导致状态不一致）
        let provider = task.llm.provider.clone();
        let model = task.llm.model.clone();
        let llm = self.llm.get_dyn(&provider)?;

        let cfg = ReactConfig {
            max_iterations: task.max_iterations,
            temperature: task.llm.temperature,
            top_p: task.llm.top_p,
            max_tokens: task.llm.max_tokens,
            timeout: Duration::from_secs(task.timeout_secs),
            provider,
            model,
        };

        let mut react_rx = run_react(
            llm,
            Arc::clone(&self.tools),
            tool_specs,
            messages,
            cfg,
            self.channel_buffer,
        );

        // 将 ReactEvent 转换为 LlmDelta 并转发。工具事件用 marker JSON 包装到 Text 中，
        // SSE 层拦截重新路由到专属 event（见 api/stream.rs）。
        let (fwd_tx, fwd_rx) = mpsc::channel::<LlmDelta>(self.channel_buffer);
        let memory = Arc::clone(&self.memory);
        let persist = task.memory.enabled && task.memory.max_turns > 0;
        let app_id = input.app_id.clone();
        let user_id = input.user_id.clone();
        let session_id = input.session_id.clone();
        let save_user = user_msg.clone();
        tokio::spawn(async move {
            let mut buf = String::new();
            while let Some(ev) = react_rx.recv().await {
                let delta = match ev {
                    ReactEvent::Text(t) => {
                        buf.push_str(&t);
                        LlmDelta::Text(t)
                    }
                    ReactEvent::ToolCall(c) => {
                        // 用 marker 包装 tool_call（SSE 层重新路由）
                        // 显式 Serialize 派生比 serde_json::json! 宏更高效（跳过 Value 中间表示）
                        let payload = serde_json::to_string(&ToolCallMarker::new(
                            &c.id, &c.name, &c.arguments,
                        ))
                        .unwrap_or_default();
                        LlmDelta::Text(payload)
                    }
                    ReactEvent::ToolResult(r) => {
                        let payload = serde_json::to_string(&ToolResultMarker::new(
                            &r.call_id, &r.name, r.is_error, &r.content,
                        ))
                        .unwrap_or_default();
                        LlmDelta::Text(payload)
                    }
                    ReactEvent::Done => LlmDelta::Done,
                    ReactEvent::Error(e) => LlmDelta::Error(e),
                };
                let is_terminal = matches!(delta, LlmDelta::Done | LlmDelta::Error(_));
                if fwd_tx.send(delta).await.is_err() {
                    break;
                }
                if is_terminal {
                    break;
                }
            }
            drop(fwd_tx);
            if persist && !buf.is_empty() {
                let assistant = ChatMessage::assistant(buf);
                if let Err(e) = memory
                    .append(&app_id, &user_id, &session_id, &save_user, &assistant)
                    .await
                {
                    tracing::warn!(err = %e, "memory append failed");
                }
            }
        });

        Ok(DeltaStream::new(ReceiverStream::new(fwd_rx)))
    }

    pub fn llm(&self) -> &Arc<LlmPool> {
        &self.llm
    }
    pub fn memory(&self) -> &Arc<dyn SessionStore> {
        &self.memory
    }
    pub fn config(&self) -> &ConfigHandle {
        &self.cfg
    }
    pub fn tasks(&self) -> &Arc<TaskRegistry> {
        &self.tasks
    }

    #[inline]
    pub fn task_of(&self, name: &str) -> Option<Arc<TaskConfig>> {
        self.tasks.get(name)
    }
}

/// 极简模板渲染：仅替换 `{{content}}`。避免引入 handlebars 等运行时开销。
fn render_template(tmpl: &str, content: &str) -> String {
    if tmpl.is_empty() {
        return content.to_string();
    }
    tmpl.replace("{{content}}", content)
}

/// 合并 system prompt：skill.instruction 在前，task.prompt.system 在后。
fn combine_system(task_system: &str, skill_instr: Option<&str>) -> String {
    match (skill_instr.unwrap_or("").trim(), task_system.trim()) {
        ("", "") => String::new(),
        ("", t) => t.to_string(),
        (s, "") => s.to_string(),
        (s, t) => format!("{s}\n\n{t}"),
    }
}

pin_project! {
    /// 使 `ReceiverStream` 对外暴露为一个具体的、可命名的 Stream 类型。
    pub struct DeltaStream<S> {
        #[pin]
        inner: S,
    }
}

impl<S> DeltaStream<S> {
    fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S> Stream for DeltaStream<S>
where
    S: Stream<Item = LlmDelta>,
{
    type Item = LlmDelta;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

/// tool_call 标记序列化辅助（避免 serde_json::json! 宏的 Value 中间表示开销）。
#[derive(serde::Serialize)]
struct ToolCallMarker<'a> {
    #[serde(rename = "__claw_event")]
    event: &'static str,
    id: &'a str,
    name: &'a str,
    arguments: &'a serde_json::Value,
}

impl<'a> ToolCallMarker<'a> {
    fn new(id: &'a str, name: &'a str, arguments: &'a serde_json::Value) -> Self {
        Self { event: "tool_call", id, name, arguments }
    }
}

/// tool_result 标记序列化辅助。
#[derive(serde::Serialize)]
struct ToolResultMarker<'a> {
    #[serde(rename = "__claw_event")]
    event: &'static str,
    call_id: &'a str,
    name: &'a str,
    is_error: bool,
    content: &'a str,
}

impl<'a> ToolResultMarker<'a> {
    fn new(call_id: &'a str, name: &'a str, is_error: bool, content: &'a str) -> Self {
        Self { event: "tool_result", call_id, name, is_error, content }
    }
}
