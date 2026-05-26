//! `clawctl llm` —— LLM 调试子命令。
//!
//! 实现：
//! - 加载 `config_dir/config.yaml` 拿 `LlmConfig` + `CircuitBreakerConfig`
//! - 用 [`claw_llm::LlmPool::build`] 装配池
//! - `chat` / `stream` 都走 SSE 流，`chat` 收齐后一次性 print，`stream` 边流边打印
//! - `ping` 仅探活一次（极短 max_tokens）

use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::Subcommand;
use claw_llm::{ChatMessage, LlmDelta, LlmRequest};
use claw_llm::LlmPool;
use colored::Colorize;
use std::io::Write;

use super::Ctx;
use crate::yaml_cfg;

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 单轮对话（非流式打印结果）
    Chat {
        /// LLM provider 名（缺省取 config 的 default_provider）
        #[arg(long)]
        provider: Option<String>,
        /// 模型（缺省取 provider 的 default_model）
        #[arg(long)]
        model: Option<String>,
        /// 用户消息
        #[arg(short, long)]
        message: String,
        /// 系统提示
        #[arg(long, default_value = "You are a helpful assistant.")]
        system: String,
        /// 最大输出 token 数
        #[arg(long, default_value_t = 512)]
        max_tokens: u32,
        /// 温度
        #[arg(long, default_value_t = 0.7)]
        temperature: f32,
    },
    /// 流式打印 token
    Stream {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "You are a helpful assistant.")]
        system: String,
        #[arg(long, default_value_t = 512)]
        max_tokens: u32,
        #[arg(long, default_value_t = 0.7)]
        temperature: f32,
    },
    /// 探活（验证 endpoint + API key）
    Ping {
        #[arg(long)]
        provider: Option<String>,
    },
}

pub async fn run(ctx: &Ctx, sub: Sub) -> anyhow::Result<()> {
    if ctx.llm_mock {
        eprintln!(
            "{}",
            "[mock] llm subcommand ignores --llm-mock and still calls real provider; \
             pass real keys via env (e.g. OPENAI_API_KEY)."
                .yellow()
        );
    }

    let cfg = yaml_cfg::load_app_config(&ctx.config_dir)
        .with_context(|| format!("load config from {}", ctx.config_dir.display()))?;

    let pool = LlmPool::build(&cfg.llm, &cfg.circuit_breaker, 256)
        .map_err(|e| anyhow!("build LlmPool: {e}"))?;

    match sub {
        Sub::Chat {
            provider,
            model,
            message,
            system,
            max_tokens,
            temperature,
        } => {
            let (provider, model) = resolve_provider_model(&cfg, provider, model)?;
            let req = build_request(&provider, &model, &system, &message, temperature, max_tokens);
            let client = pool.get(&provider).map_err(|e| anyhow!("{e}"))?;
            let mut rx = client
                .chat_stream(req)
                .await
                .map_err(|e| anyhow!("chat_stream: {e}"))?;
            let mut buf = String::new();
            while let Some(d) = rx.recv().await {
                match d {
                    LlmDelta::Text(t) => buf.push_str(&t),
                    LlmDelta::ToolCalls(calls) => {
                        eprintln!(
                            "{} {} tool_call(s) returned",
                            "[note]".yellow(),
                            calls.len()
                        );
                    }
                    LlmDelta::Done => break,
                    LlmDelta::Error(e) => {
                        return Err(anyhow!("stream error: {e}"));
                    }
                }
            }
            println!("{}", buf);
        }

        Sub::Stream {
            provider,
            model,
            message,
            system,
            max_tokens,
            temperature,
        } => {
            let (provider, model) = resolve_provider_model(&cfg, provider, model)?;
            let req = build_request(&provider, &model, &system, &message, temperature, max_tokens);
            let client = pool.get(&provider).map_err(|e| anyhow!("{e}"))?;
            let mut rx = client
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
                    LlmDelta::ToolCalls(calls) => {
                        eprintln!(
                            "\n{} {} tool_call(s)",
                            "[note]".yellow(),
                            calls.len()
                        );
                    }
                    LlmDelta::Done => break,
                    LlmDelta::Error(e) => {
                        return Err(anyhow!("stream error: {e}"));
                    }
                }
            }
            let _ = out.write_all(b"\n");
        }

        Sub::Ping { provider } => {
            let (provider, model) = resolve_provider_model(&cfg, provider, None)?;
            let req = build_request(&provider, &model, "", "ping", 0.0, 4);
            let client = pool.get(&provider).map_err(|e| anyhow!("{e}"))?;
            let started = std::time::Instant::now();
            let mut rx = client
                .chat_stream(req)
                .await
                .map_err(|e| anyhow!("chat_stream: {e}"))?;
            let mut got_any = false;
            while let Some(d) = rx.recv().await {
                match d {
                    LlmDelta::Text(_) | LlmDelta::ToolCalls(_) => got_any = true,
                    LlmDelta::Done => break,
                    LlmDelta::Error(e) => {
                        return Err(anyhow!("stream error: {e}"));
                    }
                }
            }
            let elapsed = started.elapsed();
            println!(
                "{} provider={} model={} got_any={} elapsed={}ms",
                if got_any { "OK".green().bold() } else { "OK(empty)".yellow().bold() },
                provider,
                model,
                got_any,
                elapsed.as_millis(),
            );
        }
    }
    Ok(())
}

fn resolve_provider_model(
    cfg: &yaml_cfg::AppConfigLite,
    provider: Option<String>,
    model: Option<String>,
) -> anyhow::Result<(String, String)> {
    let provider = provider.unwrap_or_else(|| cfg.llm.default_provider.clone());
    let p = cfg
        .llm
        .providers
        .get(&provider)
        .ok_or_else(|| anyhow!("provider `{provider}` not in config"))?;
    let model = model.unwrap_or_else(|| p.default_model.clone());
    Ok((provider, model))
}

fn build_request(
    provider: &str,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    max_tokens: u32,
) -> LlmRequest {
    let mut messages = Vec::with_capacity(2);
    if !system.is_empty() {
        messages.push(ChatMessage::system(system));
    }
    messages.push(ChatMessage::user(user));
    LlmRequest {
        provider: provider.to_string(),
        model: model.to_string(),
        messages,
        temperature,
        top_p: 0.9,
        max_tokens,
        stream: true,
        timeout: Duration::from_secs(60),
        tools: Vec::new(),
    }
}
