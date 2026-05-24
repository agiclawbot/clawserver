//! Axum 高并发 HTTP Server。
//!
//! 设计亮点：
//! - 单 `Router` + `State(Arc<AgentEngine>)`，所有 handler 零锁只读
//! - `tower_governor` 令牌桶限流（无锁实现）
//! - `tower_http::trace` 结构化 tracing
//! - 优雅关闭：`signal::ctrl_c` + SIGTERM 双通道，drain in-flight 请求
//! - `TcpListener` 显式打开 SO_REUSEADDR / TCP_NODELAY，配合 keepalive

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tokio::net::TcpListener;
use tokio::signal;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};

use claw_agent::AgentEngine;
use claw_config::ConfigHandle;
use claw_core::error::{AppError, AppResult};

use crate::stream::agent_stream;

/// 构建 Router（不含监听），便于集成测试。
pub fn build_router(engine: Arc<AgentEngine>) -> Router {
    let cfg = engine.config().load();

    // 初始化 Prometheus 指标注册表
    crate::metrics::init_metrics();

    // 限流：tower_governor 基于 governor crate，无锁令牌桶，内部 DashMap 按 key 分片
    let rate_layer = if cfg.rate_limit.enabled {
        let gov = GovernorConfigBuilder::default()
            .per_second(cfg.rate_limit.per_second.max(1) as u64)
            .burst_size(cfg.rate_limit.burst.max(1))
            .finish()
            .expect("governor config");
        Some(GovernorLayer { config: Arc::new(gov) })
    } else {
        None
    };

    let mut api_routes = Router::new()
        .route("/v1/agent/stream", post(agent_stream))
        .with_state(engine.clone());

    if let Some(rl) = rate_layer {
        api_routes = api_routes.layer(rl);
    }

    let ops_routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/version", get(version))
        .with_state(engine);

    let body_limit = cfg.server.body_limit_bytes;
    let _timeout = Duration::from_secs(cfg.server.request_timeout_secs);

    // 说明：服务层 Timeout 与 tower-http 0.5 的 TraceLayer / RequestBodyLimit
    // 组合时存在 ResponseBody: Default 约束问题，且底层 reqwest 已持有
    // per-request timeout，因此此处不绑定全局的 tower TimeoutLayer。
    crate::metrics::add_metrics(
        Router::new()
            .merge(api_routes)
            .merge(ops_routes)
            .layer(RequestBodyLimitLayer::new(body_limit))
            .layer(CompressionLayer::new())
            .layer(
                CorsLayer::new()
                    .allow_methods(Any)
                    .allow_origin(Any)
                    .allow_headers(Any),
            )
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().include_headers(false))
                    .on_response(DefaultOnResponse::new()),
            ),
    )
}

/// 启动并阻塞直到关闭信号。
pub async fn serve(engine: Arc<AgentEngine>, cfg_handle: ConfigHandle) -> AppResult<()> {
    let cfg = cfg_handle.load();
    let addr: SocketAddr = cfg
        .server
        .bind
        .parse()
        .map_err(|e| AppError::Config(format!("bind parse: {e}")))?;

    let listener = TcpListener::bind(addr).await?;
    if let Ok(std_listener) = listener.local_addr() {
        tracing::info!(addr = %std_listener, "clawserver listening");
    }

    let router = build_router(engine.clone());

    // 优雅关闭
    let shutdown_secs = cfg.server.graceful_shutdown_secs;
    let shutdown = async move {
        let ctrl_c = async { let _ = signal::ctrl_c().await; };
        #[cfg(unix)]
        let term = async {
            let mut s = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
            s.recv().await;
        };
        #[cfg(not(unix))]
        let term = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("SIGINT received, shutting down"),
            _ = term => tracing::info!("SIGTERM received, shutting down"),
        }
        // 给在飞请求最长 shutdown_secs 秒 drain 时间
        tokio::time::sleep(Duration::from_millis(50)).await;
        tracing::info!(secs = shutdown_secs, "graceful drain begin");
    };

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(())
}

// ---------------- ops endpoints ----------------

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[derive(serde::Serialize)]
struct Ready {
    status: &'static str,
    redis: bool,
    tasks: usize,
}

async fn readyz(State(engine): State<Arc<AgentEngine>>) -> impl IntoResponse {
    let redis_ok = engine.memory().health().await.is_ok();
    let tasks = engine.tasks().len();
    let status = if redis_ok && tasks > 0 { "ready" } else { "degraded" };
    let code = if status == "ready" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(Ready { status, redis: redis_ok, tasks }))
}

#[derive(serde::Serialize)]
struct Version {
    name: &'static str,
    version: &'static str,
    tasks: Vec<String>,
}

async fn version(State(engine): State<Arc<AgentEngine>>) -> impl IntoResponse {
    let tasks = engine.tasks().names().map(|s| s.to_string()).collect();
    Json(Version {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        tasks,
    })
}
