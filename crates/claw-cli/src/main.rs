//! clawctl：ClawServer 命令行调试工具。
//!
//! # 子命令树（骨架阶段，stub 实现）
//! ```text
//! clawctl
//! ├── server     启动 HTTP 服务（Step 8 联通）
//! ├── llm        LLM 调试 (chat / stream / ping)
//! ├── tool       工具调试 (list / spec / invoke)
//! ├── skill      Skill 调试 (list / show / validate)
//! ├── agent      Agent 端到端 (run / trace / replay)
//! ├── config     配置调试 (show / validate / tasks)
//! └── bench      内置 micro-bench
//! ```
//!
//! 当前所有子命令打印 `[TODO] not implemented yet`，后续阶段逐步联通。

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
