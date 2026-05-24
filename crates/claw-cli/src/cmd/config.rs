//! `clawctl config` —— 配置调试子命令。
//!
//! - `show`: 把精简 AppConfig 序列化为 yaml 打印，敏感字段脱敏
//! - `validate`: 仅尝试加载 + 内置校验，成功则退出码 0
//! - `tasks`: 列出所有 enabled 的 task

use clap::Subcommand;
use colored::Colorize;
use serde::Serialize;

use super::Ctx;
use crate::yaml_cfg;

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 打印当前生效的完整配置（敏感 env 名脱敏）
    Show,
    /// 校验 yaml schema 合法性
    Validate,
    /// 列出已加载的 task 列表
    Tasks,
}

pub async fn run(ctx: &Ctx, sub: Sub) -> anyhow::Result<()> {
    match sub {
        Sub::Show => {
            let cfg = yaml_cfg::load_app_config(&ctx.config_dir)?;
            // serde_yaml 序列化（YAML 比 JSON 更接近原始配置 vibes）。
            let printable = ShowView::from(&cfg);
            let yaml = serde_yaml::to_string(&printable)
                .map_err(|e| anyhow::anyhow!("yaml serialize: {e}"))?;
            println!("{yaml}");
        }
        Sub::Validate => {
            match yaml_cfg::load_app_config(&ctx.config_dir) {
                Ok(cfg) => {
                    println!(
                        "{} providers={} tasks={}",
                        "OK".green().bold(),
                        cfg.llm.providers.len(),
                        cfg.tasks.len()
                    );
                }
                Err(e) => {
                    eprintln!("{} {}", "FAIL".red().bold(), e);
                    std::process::exit(2);
                }
            }
        }
        Sub::Tasks => {
            let cfg = yaml_cfg::load_app_config(&ctx.config_dir)?;
            let mut names: Vec<&String> = cfg.tasks.keys().collect();
            names.sort();
            println!("{} task(s) loaded:", names.len());
            for n in names {
                let t = cfg.tasks.get(n).unwrap();
                println!(
                    "  {} [{:?}] provider={} model={} tools={}",
                    n.green(),
                    t.mode,
                    t.llm.provider,
                    t.llm.model,
                    t.tools.len()
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Show view：仅展示用，不暴露 api_key 实际值（只暴露 env 变量名）。
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ShowView<'a> {
    llm: LlmShow<'a>,
    circuit_breaker: &'a claw_core::config::CircuitBreakerConfig,
    tasks: Vec<TaskShow<'a>>,
}

#[derive(Serialize)]
struct LlmShow<'a> {
    default_provider: &'a str,
    providers: std::collections::BTreeMap<&'a str, ProviderShow<'a>>,
    retry: &'a claw_core::config::RetryConfig,
}

#[derive(Serialize)]
struct ProviderShow<'a> {
    base_url: &'a str,
    api_key_env: &'a str,
    default_model: &'a str,
    api_key_set: bool,
    pool_idle_per_host: usize,
    pool_max_idle_secs: u64,
    request_timeout_secs: u64,
    connect_timeout_secs: u64,
}

#[derive(Serialize)]
struct TaskShow<'a> {
    name: &'a str,
    mode: yaml_cfg::TaskModeLite,
    provider: &'a str,
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    tools: &'a [String],
    skill: Option<&'a str>,
    max_iterations: u32,
}

impl<'a> ShowView<'a> {
    fn from(cfg: &'a yaml_cfg::AppConfigLite) -> Self {
        let providers = cfg
            .llm
            .providers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str(),
                    ProviderShow {
                        base_url: &v.base_url,
                        api_key_env: &v.api_key_env,
                        default_model: &v.default_model,
                        api_key_set: !std::env::var(&v.api_key_env).unwrap_or_default().is_empty(),
                        pool_idle_per_host: v.pool_idle_per_host,
                        pool_max_idle_secs: v.pool_max_idle_secs,
                        request_timeout_secs: v.request_timeout_secs,
                        connect_timeout_secs: v.connect_timeout_secs,
                    },
                )
            })
            .collect();

        let mut tasks: Vec<TaskShow> = cfg
            .tasks
            .iter()
            .map(|(_, t)| TaskShow {
                name: &t.name,
                mode: t.mode,
                provider: &t.llm.provider,
                model: &t.llm.model,
                max_tokens: t.llm.max_tokens,
                temperature: t.llm.temperature,
                tools: &t.tools,
                skill: t.skill.as_deref(),
                max_iterations: t.max_iterations,
            })
            .collect();
        tasks.sort_by(|a, b| a.name.cmp(b.name));

        Self {
            llm: LlmShow {
                default_provider: &cfg.llm.default_provider,
                providers,
                retry: &cfg.llm.retry,
            },
            circuit_breaker: &cfg.circuit_breaker,
            tasks,
        }
    }
}
