//! # clawctl：ClawServer CLI 调试工具
//!
//! 独立于 HTTP 服务的命令行客户端，直接调用 LLM / Tool / Agent 等子系统，
//! **不依赖 Redis**（Agent 的 ReAct 循环使用进程内状态）。
//!
//! ## 子命令
//!
//! | 命令 | 功能 | 需要配置 |
//! |------|------|----------|
//! | `llm chat` | 直接调用 LLM，测试 prompt / 模型 / 参数 | `config.yaml` |
//! | `tool list/spec/invoke` | 工具注册表查询 + 调用测试 | `config.yaml` |
//! | `agent run/trace` | 端到端 Agent 调试（Plain / ReAct） | `config.yaml` + `config/tasks/*.yaml` |
//! | `config show` | 查看当前配置摘要 | `config.yaml` |
//! | `skill list/show/validate` | Skill 系统调试 | `config/skills/*/` |
//! | `server` | 启动完整 HTTP 服务（同 root bin） | `config.yaml` |
//! | `bench` | 内置 micro-benchmark | — |
//!
//! ## 设计原则
//!
//! - **零 Redis 依赖**：调试时不需要启动 Redis
//! - **最小 YAML 加载**：用 `yaml_cfg::AppConfigLite` 只解析需要的字段
//! - **独立 ReAct**：cli 自带最小 ReAct 循环，不依赖 `claw_agent`，避免引入 Redis 依赖

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use colored::Colorize;

mod builtin;
mod cmd;
mod yaml_cfg;

#[derive(Parser, Debug)]
#[command(
    name = "clawctl",
    version,
    about = "ClawServer 命令行调试工具",
    long_about = "用于独立调试 ClawServer 各模块：LLM / Tool / Skill / Agent / Config。\n\
                  无需启动 HTTP 服务即可单独触发任一模块。"
)]
struct Cli {
    /// 配置目录（默认 ./config，可用环境变量 CLAW_CONFIG_DIR 覆盖）
    #[arg(long, global = true, env = "CLAW_CONFIG_DIR", default_value = "config")]
    config: PathBuf,

    /// 强制使用 mock LLM（不发真实 HTTP）
    #[arg(long, global = true)]
    llm_mock: bool,

    /// 日志级别（trace / debug / info / warn / error）
    #[arg(long, global = true, env = "RUST_LOG", default_value = "info")]
    log: String,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// 启动 HTTP 服务（等价于现有 clawserver bin）
    Server(cmd::server::Args),

    /// LLM 调试（chat / stream / ping）
    Llm {
        #[command(subcommand)]
        sub: cmd::llm::Sub,
    },

    /// 工具调试（list / spec / invoke）
    Tool {
        #[command(subcommand)]
        sub: cmd::tool::Sub,
    },

    /// Skill 调试（list / show / validate）
    Skill {
        #[command(subcommand)]
        sub: cmd::skill::Sub,
    },

    /// Agent 端到端调试（run / trace / replay）
    Agent {
        #[command(subcommand)]
        sub: cmd::agent::Sub,
    },

    /// 配置调试（show / validate / tasks）
    Config {
        #[command(subcommand)]
        sub: cmd::config::Sub,
    },

    /// 内置 micro-bench
    Bench {
        #[command(subcommand)]
        sub: cmd::bench::Sub,
    },
}

fn init_tracing(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log);

    if cli.llm_mock {
        eprintln!("{}", "[mock] LLM mock mode enabled".yellow());
    }

    let ctx = cmd::Ctx {
        config_dir: cli.config.clone(),
        llm_mock: cli.llm_mock,
    };

    match cli.cmd {
        Cmd::Server(args) => cmd::server::run(&ctx, args).await,
        Cmd::Llm { sub } => cmd::llm::run(&ctx, sub).await,
        Cmd::Tool { sub } => cmd::tool::run(&ctx, sub).await,
        Cmd::Skill { sub } => cmd::skill::run(&ctx, sub).await,
        Cmd::Agent { sub } => cmd::agent::run(&ctx, sub).await,
        Cmd::Config { sub } => cmd::config::run(&ctx, sub).await,
        Cmd::Bench { sub } => cmd::bench::run(&ctx, sub).await,
    }
}
