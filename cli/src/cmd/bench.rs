//! `clawctl bench` —— 内置 micro-bench。
//!
//! - `bench tool`：工具调用吞吐（纯本地，不依赖 LLM）
//! - `bench react`：LLM 端到端 ReAct 延迟

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Subcommand;
use claw_llm::{ChatMessage, ChatProvider, LlmDelta, LlmRequest};
use claw_llm::LlmPool;

use super::Ctx;
use crate::builtin::build_default_registry;
use crate::yaml_cfg;

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 工具调用吞吐 bench
    Tool {
        /// 迭代次数
        #[arg(long, default_value = "1000")]
        iters: u64,
    },
    /// ReAct 单次平均时延（需配置 LLM）
    React {
        #[arg(long, default_value = "10")]
        iters: u64,
    },
}

pub async fn run(ctx: &Ctx, sub: Sub) -> Result<()> {
    match sub {
        Sub::Tool { iters } => bench_tool(iters).await,
        Sub::React { iters } => bench_react(ctx, iters).await,
    }
}

// ---------------------------------------------------------------------------
// bench tool
// ---------------------------------------------------------------------------

async fn bench_tool(iters: u64) -> Result<()> {
    let registry = build_default_registry();
    let echo = registry
        .get("echo")
        .context("echo tool not registered")?;

    let args = serde_json::json!({"text": "bench"});
    let warmup = 10.min(iters);
    for _ in 0..warmup {
        let _ = echo.invoke(args.clone()).await?;
    }

    let mut samples = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let start = Instant::now();
        let _ = echo.invoke(args.clone()).await?;
        samples.push(start.elapsed());
    }

    print_stats("tool/echo", &samples, iters);
    Ok(())
}

// ---------------------------------------------------------------------------
// bench react
// ---------------------------------------------------------------------------

async fn bench_react(ctx: &Ctx, iters: u64) -> Result<()> {
    let cfg = yaml_cfg::load_app_config(&ctx.config_dir)
        .with_context(|| format!("load config from {}", ctx.config_dir.display()))?;

    let pool = LlmPool::build(&cfg.llm, &cfg.circuit_breaker, 256)
        .map_err(|e| anyhow::anyhow!("build LlmPool: {e}"))?;

    let provider_name = &cfg.llm.default_provider;
    let provider = pool
        .get_dyn(provider_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // 找一个配置中的 model
    let provider_cfg = cfg
        .llm
        .providers
        .get(provider_name)
        .context("provider not found in config")?;
    let model = &provider_cfg.default_model;

    // 无工具，要求 LLM 立即回复 "done"
    let sys = ChatMessage::system("You are a helpful assistant. Respond as concisely as possible.");
    let user = ChatMessage::user("Reply with exactly the word: done");

    let warmup_iters = 1.min(iters);
    for _ in 0..warmup_iters {
        let _ = single_round_trip(&*provider, &model, &sys, &user).await?;
    }

    let mut samples = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let start = Instant::now();
        let _ = single_round_trip(&*provider, &model, &sys, &user).await?;
        samples.push(start.elapsed());
    }

    println!("\n── bench react ──");
    println!("  provider : {provider_name}");
    println!("  model    : {model}");
    print_stats("react/llm-roundtrip", &samples, iters);
    Ok(())
}

/// 单次 LLM 请求，流式收集完返回。
async fn single_round_trip(
    provider: &dyn ChatProvider,
    model: &str,
    sys: &ChatMessage,
    user: &ChatMessage,
) -> Result<String> {
    let req = LlmRequest {
        provider: "bench".into(),
        model: model.into(),
        messages: vec![sys.clone(), user.clone()],
        temperature: 0.0,
        top_p: 1.0,
        max_tokens: 10,
        stream: true,
        timeout: Duration::from_secs(30),
        tools: Vec::new(),
    };
    let mut rx = provider.chat_stream(req).await.map_err(|e| anyhow::anyhow!("chat_stream: {e}"))?;
    let mut text = String::new();
    while let Some(d) = rx.recv().await {
        match d {
            LlmDelta::Text(t) => text.push_str(&t),
            LlmDelta::Done => break,
            LlmDelta::ToolCalls(_) => {}
            LlmDelta::Error(e) => return Err(anyhow::anyhow!("stream error: {e}")),
        }
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// 统计输出
// ---------------------------------------------------------------------------

fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 1.0 {
        format!("{secs:.3}s")
    } else if secs >= 0.001 {
        format!("{:.3}ms", secs * 1_000.0)
    } else {
        format!("{:.1}µs", secs * 1_000_000.0)
    }
}

fn print_stats(label: &str, samples: &[Duration], iters: u64) {
    let total: Duration = samples.iter().sum();
    let min = samples.iter().min().copied().unwrap_or_default();
    let max = samples.iter().max().copied().unwrap_or_default();
    let avg = total / u32::try_from(samples.len()).unwrap_or(1);
    let per_sec = if total.is_zero() {
        0.0
    } else {
        iters as f64 / total.as_secs_f64()
    };

    println!("\n── bench {label} ──");
    println!("  iters    : {iters}");
    println!("  total    : {}", fmt_duration(total));
    println!("  min      : {}", fmt_duration(min));
    println!("  avg      : {}", fmt_duration(avg));
    println!("  max      : {}", fmt_duration(max));
    println!("  ops/sec  : {per_sec:.1}");
}
