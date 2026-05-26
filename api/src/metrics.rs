//! Prometheus 指标收集与导出。
//!
//! - HTTP 请求数、耗时、活跃连接数的 axum middleware
//! - `/metrics` 路由暴露 Prometheus 文本格式

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use prometheus::{
    register_counter_vec_with_registry, register_gauge_vec_with_registry,
    register_histogram_vec_with_registry, CounterVec, GaugeVec, HistogramOpts,
    HistogramVec, Opts, Registry, TextEncoder,
};

static METRICS: OnceLock<Arc<AppMetrics>> = OnceLock::new();

/// 获取全局指标实例。
pub fn global() -> &'static Arc<AppMetrics> {
    METRICS.get().expect("metrics not initialized; call init_metrics() first")
}

pub struct AppMetrics {
    pub registry: Registry,

    /// HTTP 请求总数（method, path, status）
    http_requests_total: CounterVec,
    /// HTTP 请求耗时秒数（method, path）
    http_request_duration_seconds: HistogramVec,
    /// 处理中的 HTTP 请求数（method）
    http_requests_in_flight: GaugeVec,
}

impl AppMetrics {
    fn new() -> Self {
        let registry = Registry::new();

        let http_requests_total = register_counter_vec_with_registry!(
            Opts::new("http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
            registry,
        )
        .unwrap();

        let http_request_duration_seconds = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request duration in seconds",
            ),
            &["method", "path"],
            registry,
        )
        .unwrap();

        let http_requests_in_flight = register_gauge_vec_with_registry!(
            Opts::new("http_requests_in_flight", "In-flight HTTP requests"),
            &["method"],
            registry,
        )
        .unwrap();

        Self {
            registry,
            http_requests_total,
            http_request_duration_seconds,
            http_requests_in_flight,
        }
    }

    pub fn observe(&self, method: &str, path: &str, status: u16, duration_secs: f64) {
        self.http_requests_total
            .with_label_values(&[method, path, &status.to_string()])
            .inc();
        self.http_request_duration_seconds
            .with_label_values(&[method, path])
            .observe(duration_secs);
    }

    pub fn inc_in_flight(&self, method: &str) {
        self.http_requests_in_flight
            .with_label_values(&[method])
            .inc();
    }

    pub fn dec_in_flight(&self, method: &str) {
        self.http_requests_in_flight
            .with_label_values(&[method])
            .dec();
    }
}

/// 初始化全局指标注册表（幂等）。
pub fn init_metrics() -> &'static Arc<AppMetrics> {
    METRICS.get_or_init(|| Arc::new(AppMetrics::new()))
}

/// GET /metrics — Prometheus 文本格式。
pub async fn metrics_handler() -> (axum::http::StatusCode, String) {
    let metrics = global();
    let encoder = TextEncoder::new();
    let mut buf = String::new();
    match encoder.encode_utf8(&metrics.registry.gather(), &mut buf) {
        Ok(_) => (axum::http::StatusCode::OK, buf),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("encode error: {e}")),
    }
}

/// axum middleware：记录每个 HTTP 请求的 method / path / status / duration。
pub async fn metrics_middleware(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let metrics = global();
    metrics.inc_in_flight(&method);

    let resp = next.run(req).await;

    metrics.dec_in_flight(&method);
    metrics.observe(&method, &path, resp.status().as_u16(), start.elapsed().as_secs_f64());

    resp
}

/// 把 metrics 路由和 middleware 添加到 Router。
pub fn add_metrics(router: axum::Router) -> axum::Router {
    router
        .route("/metrics", axum::routing::get(metrics_handler))
        .route_layer(axum::middleware::from_fn(metrics_middleware))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        let a = init_metrics();
        let b = init_metrics();
        assert!(std::ptr::eq(Arc::as_ptr(a), Arc::as_ptr(b)));
    }

    #[test]
    fn observe_increments_counter() {
        let m = init_metrics();
        m.observe("POST", "/v1/agent/stream", 200, 0.5);
        let gathered = m.registry.gather();
        assert!(gathered.iter().any(|mf| mf.get_name() == "http_requests_total"));
    }
}
