//! CLI 子命令模块注册。
//!
//! 每个子模块暴露：
//! - `Sub` / `Args`（clap 结构体）
//! - `run(ctx, args) -> anyhow::Result<()>`
//!
//! 当前全部为 stub：打印 [TODO] 后返回 Ok。后续阶段逐步联通实际 crate。

use std::path::PathBuf;

pub mod agent;
pub mod bench;
pub mod config;
pub mod llm;
pub mod server;
pub mod skill;
pub mod tool;

/// 子命令共享上下文（从 Cli 全局参数传入）。
pub struct Ctx {
    pub config_dir: PathBuf,
    pub llm_mock: bool,
}
