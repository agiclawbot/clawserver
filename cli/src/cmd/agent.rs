//! `clawctl agent` —— Agent 端到端调试子命令。
//!
//! 设计：
//! - 不依赖根 crate；cli 自己做最小 ReAct 循环，复用 [`claw_llm::LlmPool`] + [`claw_llm::ToolRegistry`]
//! - `run`：单次跑通，按 task.mode 决定走 plain 流式或 ReAct 多轮
//! - `trace`：等同 `run` 但额外打印 thought / tool_call / tool_result 详情
//! - `replay`：当前 cli 不连 Redis，给出友好提示，留 server 侧未来联通

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use claw_llm::{AssistantToolCall, ChatMessage, ChatProvider, LlmDelta, LlmRequest};
use claw_llm::{ToolCall, ToolRegistry, ToolResult, ToolSpec};
use claw_llm::LlmPool;
use colored::Colorize;
use std::io::Write;

use super::Ctx;
use crate::builtin::build_default_registry;
use crate::yaml_cfg::{self, TaskConfigLite, TaskModeLite};

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 单次跑通（输出最终文本）
    Run {
        /// 任务类型（对应 config/tasks/<name>.yaml 的 name）
        #[arg(long)]
        task: String,
        /// 用户输入
        #[arg(short, long)]
        content: String,
    },
    /// 带 thought/tool_call/tool_result 详细打印
    Trace {
        #[arg(long)]
        task: String,
        #[arg(short, long)]
        content: String,
        /// 输出格式 (pretty / json)
        #[arg(long, default_value = "pretty")]
        format: String,
    },
    /// 从 Redis 加载会话回放（需要服务端 ; cli 暂不支持）
    Replay {
        /// session_id
        session_id: String,
    },
}

pub async fn run(ctx: &Ctx, sub: Sub) -> Result<()> {
    match sub {
        Sub::Run { task, content } => execute(ctx, &task, &content, false, "pretty").await,
        Sub::Trace { task, content, format } => {
            execute(ctx, &task, &content, true, &format).await
        }
        Sub::Replay { session_id } => {
            eprintln!(
                "{} cli does not connect Redis. To replay session `{session_id}`, \
                 inspect Redis directly or use the server-side endpoint.",
                "[note]".yellow()
            );
            Ok(())
        }
    }
}

async fn execute(
    ctx: &Ctx,
    task_name: &str,
    content: &str,
    trace: bool,
    format: &str,
) -> Result<()> {
    let cfg = yaml_cfg::load_app_config(&ctx.config_dir)
        .with_context(|| format!("load config from {}", ctx.config_dir.display()))?;
    let task = cfg
        .tasks
        .get(task_name)
        .ok_or_else(|| anyhow!("task `{task_name}` not found in {}", ctx.config_dir.display()))?
        .clone();

    let pool = LlmPool::build(&cfg.llm, &cfg.circuit_breaker, 256)
        .map_err(|e| anyhow!("build LlmPool: {e}"))?;
    let provider = pool
        .get_dyn(&task.llm.provider)
        .map_err(|e| anyhow!("{e}"))?;

    let registry = build_default_registry();

    let messages = build_initial_messages(&ctx.config_dir, &task, content)?;

    match task.mode {
        TaskModeLite::Plain => run_plain(provider, &task, messages, trace, format).await,
        TaskModeLite::React => {
            run_react(provider, registry, &task, messages, trace, format).await
        }
    }
}

fn build_initial_messages(
    config_dir: &std::path::Path,
    task: &TaskConfigLite,
    content: &str,
) -> Result<Vec<ChatMessage>> {
    let mut sys = task.prompt.system.clone();

    if let Some(skill_name) = &task.skill {
        let inst_path = config_dir
            .join("skills")
            .join(skill_name)
            .join("instruction.md");
        if let Ok(extra) = std::fs::read_to_string(&inst_path) {
            if !sys.is_empty() {
                sys.push_str("\n\n");
            }
            sys.push_str(&extra);
        }
    }

    let user = task.prompt.user_template.replace("{{content}}", content);

    let mut messages = Vec::with_capacity(2);
    if !sys.is_empty() {
        messages.push(ChatMessage::system(sys));
    }
    messages.push(ChatMessage::user(user));
    Ok(messages)
}

async fn run_plain(
    provider: Arc<dyn ChatProvider>,
    task: &TaskConfigLite,
    messages: Vec<ChatMessage>,
    trace: bool,
    _format: &str,
) -> Result<()> {
    if trace {
        eprintln!(
            "{} task={} mode=plain provider={} model={}",
            "[trace]".cyan(),
            task.name,
            task.llm.provider,
            task.llm.model,
        );
    }
    let req = LlmRequest {
        provider: task.llm.provider.clone(),
        model: task.llm.model.clone(),
        messages,
        temperature: task.llm.temperature,
        top_p: task.llm.top_p,
        max_tokens: task.llm.max_tokens,
        stream: true,
        timeout: Duration::from_secs(60),
        tools: Vec::new(),
    };
    let mut rx = provider
        .chat_stream(req)
        .await
        .map_err(|e| anyhow!("chat_stream: {e}"))?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    while let Some(d) = rx.recv().await {
        match d {
            LlmDelta::Text(t) => {
                let _ = out.write_all(t.as_bytes());
                let _ = out.flush();
            }
            LlmDelta::ToolCalls(_) => {
                if trace {
                    eprintln!("\n{} unexpected tool_calls in plain mode", "[trace]".cyan());
                }
            }
            LlmDelta::Done => break,
            LlmDelta::Error(e) => return Err(anyhow!("stream error: {e}")),
        }
    }
    let _ = out.write_all(b"\n");
    Ok(())
}

async fn run_react(
    provider: Arc<dyn ChatProvider>,
    registry: Arc<ToolRegistry>,
    task: &TaskConfigLite,
    initial_messages: Vec<ChatMessage>,
    trace: bool,
    _format: &str,
) -> Result<()> {
    let max_iter = task.max_iterations.max(1);
    let tool_specs: Vec<ToolSpec> = registry.specs_for(&task.tools);
    if trace {
        eprintln!(
            "{} task={} mode=react max_iter={} tools={}",
            "[trace]".cyan(),
            task.name,
            max_iter,
            tool_specs.len(),
        );
    }
    let mut messages = initial_messages;

    for iter in 0..max_iter {
        let last_round = iter + 1 == max_iter;
        let req_tools = if last_round { Vec::new() } else { tool_specs.clone() };
        let req = LlmRequest {
            provider: task.llm.provider.clone(),
            model: task.llm.model.clone(),
            messages: messages.clone(),
            temperature: task.llm.temperature,
            top_p: task.llm.top_p,
            max_tokens: task.llm.max_tokens,
            stream: true,
            timeout: Duration::from_secs(60),
            tools: req_tools,
        };
        let mut rx = provider
            .chat_stream(req)
            .await
            .map_err(|e| anyhow!("chat_stream: {e}"))?;

        let mut buf_text = String::new();
        let mut pending: Option<Vec<ToolCall>> = None;
        while let Some(d) = rx.recv().await {
            match d {
                LlmDelta::Text(t) => {
                    buf_text.push_str(&t);
                    print!("{t}");
                    let _ = std::io::stdout().flush();
                }
                LlmDelta::ToolCalls(calls) => {
                    pending = Some(calls);
                }
                LlmDelta::Done => break,
                LlmDelta::Error(e) => return Err(anyhow!("stream error: {e}")),
            }
        }

        if let Some(calls) = pending {
            println!();
            let assistant_calls: Vec<AssistantToolCall> =
                calls.iter().map(Into::into).collect();
            messages.push(ChatMessage::assistant_tool_calls(assistant_calls));

            for call in &calls {
                if trace {
                    eprintln!(
                        "{} call: {} args={}",
                        "[tool]".magenta(),
                        call.name.bold(),
                        call.arguments
                    );
                }
                let result = match registry.invoke(&call.name, call.arguments.clone()).await {
                    Ok(s) => ToolResult::ok(&call.id, &call.name, s),
                    Err(e) => ToolResult::err(&call.id, &call.name, e.to_string()),
                };
                if trace {
                    let head: String = result.content.chars().take(160).collect();
                    eprintln!(
                        "{} result ({}): {}",
                        "[tool]".magenta(),
                        if result.is_error { "ERR".red().to_string() } else { "OK".green().to_string() },
                        head
                    );
                }
                messages.push(ChatMessage::tool(&call.id, result.content.clone()));
            }
            continue;
        }

        // 没工具调用：本轮就是最终答案
        if !buf_text.is_empty() {
            messages.push(ChatMessage::assistant(buf_text));
        }
        println!();
        return Ok(());
    }

    eprintln!(
        "{} max_iterations({max_iter}) reached without final answer",
        "[warn]".yellow()
    );
    Ok(())
}
