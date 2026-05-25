//! # ClawServer 入口：纯装配。
//!
//! ## 架构分层
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                     claw-api / claw-cli                 │ ← 边界层
//! │   HTTP(S) 入口(Serve) + 路由(Router) + SSE/CLI 交互     │   (对外暴露)
//! ├─────────────────────────────────────────────────────────┤
//! │                     claw-agent                          │ ← 编排层
//! │   AgentEngine + ReAct 循环 + Session + TaskRegistry     │   (业务编排)
//! ├─────────────────────────────────────────────────────────┤
//! │      claw-llm        │    claw-config (重导出)          │ ← 基础服务层
//! │   LLM 客户端池(HTTP)  │    YAML 加载 + 校验              │   (可替换实现)
//! ├─────────────────────────────────────────────────────────┤
//! │                    claw-core                            │ ← 契约层
//! │  ChatProvider/Tool/SessionStore trait                   │
//! │  + AppConfig/AppError/工具/Skill 系统                   │   (最稳定)
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## 启动顺序（任一阶段失败立即退出）
//!
//! 1. **配置加载** — 从 `config/config.yaml` + `config/tasks/*.yaml` 读取，校验
//! 2. **tracing 初始化** — 日志格式（json/compact）+ 级别
//! 3. **tokio 运行时** — 按 CPU 核数创建多线程 runtime
//! 4. **Redis 会话池** (`SessionMemory`) — fred 异步连接，支持单机/集群/哨兵
//! 5. **LLM 连接池** (`LlmPool`) — 按 provider 分池，每个 provider 独立 reqwest Client
//! 6. **任务注册表** (`TaskRegistry`) — 从 YAML 构建，只读共享
//! 7. **工具注册表** — 内置工具 (TimeNow/HttpGet/WebSearch) 启动期注册
//! 8. **Skill 注册** — 扫描 config/skills/<name>/ 加载 instruction.md
//! 9. **AgentEngine** — 组合上述全部依赖，运行期只读
//! 10. **HTTP 服务** — axum 监听端口 + 优雅关闭 (SIGINT/SIGTERM)
//!
//! ## 扩展新功能
//!
//! - **新增 LLM 提供商**: 在 `config.yaml → llm.providers` 加一项即可
//! - **新增任务类型**: 在 `config/tasks/` 加一个 YAML 文件，0 代码
//! - **新增内置工具**: 在 `build_tool_registry()` 中 `.register()`，claw-core/tools/builtin 下实现
//! - **新增 Skill**: 在 `config/skills/<name>/` 放 manifest.yaml + instruction.md

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
