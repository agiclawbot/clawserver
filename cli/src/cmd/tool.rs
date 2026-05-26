//! `clawctl tool` —— 工具调试子命令。
//!
//! 通过 [`crate::builtin::build_default_registry`] 装配一个最小内置工具集
//! （`time_now` + `echo`），演示与 [`claw_llm::Tool`] 契约层的联通。
//!
//! 后续真实业务工具（http_get / web_search 等）可由独立的
//! `claw-tools-builtin` crate 提供，cli 通过 feature 开关引入。
use clap::Subcommand;
use colored::Colorize;

use crate::builtin::build_default_registry;

use super::Ctx;

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 列出已注册工具
    List,
    /// 打印某工具的 OpenAI tools[] JSON Schema
    Spec {
        /// 工具名
        name: String,
    },
    /// 直接调用某工具（不经 LLM）
    Invoke {
        /// 工具名
        name: String,
        /// 参数 JSON 字符串
        #[arg(long, default_value = "{}")]
        args: String,
    },
}

pub async fn run(_ctx: &Ctx, sub: Sub) -> anyhow::Result<()> {
    let registry = build_default_registry();

    match sub {
        Sub::List => {
            // 注册表内部 HashMap 无序，输出前排序，保证调试稳定
            let mut names: Vec<String> = (0..0).map(|_| String::new()).collect();
            // ToolRegistry 当前没有 iter API；用白名单方式逐个尝试名字。
            // 为了不破坏契约层，在 cli 端维护一个"已知名字"列表：
            for n in known_tool_names() {
                if registry.get(n).is_some() {
                    names.push(n.to_string());
                }
            }
            names.sort();
            println!("{}", format!("{} tool(s) registered:", names.len()).bold());
            for n in &names {
                if let Some(t) = registry.get(n) {
                    println!("  {} - {}", n.green(), t.description());
                }
            }
        }

        Sub::Spec { name } => {
            let tool = registry.get(&name).ok_or_else(|| {
                anyhow::anyhow!("tool `{name}` not found in registry")
            })?;
            let spec = tool.spec();
            let json = serde_json::to_string_pretty(&spec)?;
            println!("{json}");
        }

        Sub::Invoke { name, args } => {
            let parsed: serde_json::Value = serde_json::from_str(&args)
                .map_err(|e| anyhow::anyhow!("invalid --args JSON: {e}"))?;
            match registry.invoke(&name, parsed).await {
                Ok(out) => {
                    println!("{}", "ok:".green().bold());
                    println!("{out}");
                }
                Err(e) => {
                    eprintln!("{} {}", "error:".red().bold(), e);
                    std::process::exit(2);
                }
            }
        }
    }
    Ok(())
}

/// cli 自带的已知工具名（在 [`crate::builtin::known_tool_names`] 集中维护）。
///
/// 之所以维护此清单而非给 ToolRegistry 加 `iter()`：契约层只暴露按名查询，
/// cli 调试时显式列出可用名是更稳妥的做法（避免日后注册表内部结构变化影响 cli）。
fn known_tool_names() -> &'static [&'static str] {
    crate::builtin::known_tool_names()
}
