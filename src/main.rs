//! ClawServer 入口：纯装配。
//!
//! - 手动构造多线程 tokio 运行时，worker 数依据 config.server.worker_threads
//! - 启动顺序：config -> logging -> redis -> llm pool -> task registry -> engine -> server
//! - 任一阶段失败立即 Err 退出，避免半启动状态
//!
//! 全部业务实现已下沉到子 crate（claw-core / claw-llm / claw-agent / claw-api 等），
//! 本文件仅负责装配 + 引导。

use std::path::PathBuf;
use std::sync::Arc;

use claw_agent::{AgentEngine, SessionMemory, TaskRegistry};
use claw_core::error::{AppError, AppResult};
use claw_core::tool::ToolRegistry;
use claw_llm::LlmPool;
use claw_core::tools::builtin::{HttpGet, TimeNow, WebSearch};

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> AppResult<()> {
    // 1) 先加载配置（同步：读取 YAML）
    let config_dir = std::env::var("CLAW_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config"));
    let cfg_handle = claw_config::init_from_dir(&config_dir)?;

    // 2) 日志
    install_tracing(&cfg_handle)?;

    // 3) 构造多线程 tokio 运行时
    let workers = {
        let cfg = cfg_handle.load();
        if cfg.server.worker_threads == 0 {
            num_cpus::get()
        } else {
            cfg.server.worker_threads
        }
    };
    tracing::info!(workers, "starting tokio multi-threaded runtime");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .max_blocking_threads(512)
        .thread_name("claw-worker")
        .thread_stack_size(2 * 1024 * 1024)
        .build()
        .map_err(|e| AppError::Internal(format!("runtime build: {e}")))?;

    let config_dir_for_async = config_dir.clone();
    runtime.block_on(async_main(cfg_handle, config_dir_for_async))
}

async fn async_main(
    cfg_handle: claw_config::ConfigHandle,
    config_dir: PathBuf,
) -> AppResult<()> {
    let cfg = cfg_handle.load();

    // 3) Redis 会话池
    let memory = SessionMemory::connect(&cfg).await?;
    tracing::info!("redis connected");

    // 4) LLM 调用池
    let llm = LlmPool::build(
        &cfg.llm,
        &cfg.circuit_breaker,
        cfg.buffer.channel_size,
    )?;
    tracing::info!(providers = cfg.llm.providers.len(), "llm pool ready");

    // 5) 任务注册表
    let tasks = TaskRegistry::build(&cfg);
    tracing::info!(tasks = tasks.len(), "task registry ready");
    if tasks.is_empty() {
        tracing::warn!("no tasks loaded; POST /v1/agent/stream will return 404");
    }

    // 6) 内置工具注册（运行期只读共享）
    let tools = build_tool_registry();
    tracing::info!(tools = tools.len(), "tool registry ready");

    // 6.2) Skill 注册（扫描 config/skills/<name>/{manifest.yaml, instruction.md}）
    let skills_dir = config_dir.join("skills");
    let skills = match claw_core::skill::load_from_dir(&skills_dir) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            tracing::warn!(err = %e, "load skills failed, continue with empty registry");
            Arc::new(claw_core::skill::SkillRegistry::new())
        }
    };
    tracing::info!(skills = skills.len(), "skill registry ready");

    // 7) Agent 引擎
    let engine = AgentEngine::new(cfg_handle.clone(), tasks, memory, llm, tools, skills);

    // 8) 启动 HTTP
    claw_api::serve(engine, cfg_handle).await
}

/// 构造内置工具注册表。新增工具在此 register 即可。
fn build_tool_registry() -> Arc<ToolRegistry> {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(TimeNow));
    reg.register(Arc::new(HttpGet::new()));
    reg.register(Arc::new(WebSearch));
    Arc::new(reg)
}

fn install_tracing(cfg_handle: &claw_config::ConfigHandle) -> AppResult<()> {
    let cfg = cfg_handle.load();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.observability.log_level));

    let registry = tracing_subscriber::registry().with(filter);
    match cfg.observability.log_format.as_str() {
        "json" => {
            registry
                .with(fmt::layer().json().with_current_span(false))
                .try_init()
                .map_err(|e| AppError::Internal(format!("tracing init: {e}")))?;
        }
        _ => {
            registry
                .with(fmt::layer().compact())
                .try_init()
                .map_err(|e| AppError::Internal(format!("tracing init: {e}")))?;
        }
    }
    Ok(())
}
