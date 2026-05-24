//! 全局错误模型（纯枚举）。
//!
//! 不依赖任何 HTTP / web 框架；HTTP 映射 (`IntoResponse`) 在 server crate 内单独实现。

use thiserror::Error;

pub type AppResult<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("task `{0}` not found")]
    TaskNotFound(String),

    #[error("rate limited")]
    RateLimited,

    #[error("upstream circuit open: {0}")]
    CircuitOpen(&'static str),

    #[error("redis error: {0}")]
    Redis(String),

    #[error("llm error: {0}")]
    Llm(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde error: {0}")]
    Serde(String),

    #[error("internal: {0}")]
    Internal(String),

    #[error("timeout")]
    Timeout,
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Serde(e.to_string())
    }
}

#[cfg(feature = "yaml")]
impl From<serde_yaml::Error> for AppError {
    fn from(e: serde_yaml::Error) -> Self {
        AppError::Serde(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// axum 适配：仅在 `ax-response` feature 开启时编译
// ---------------------------------------------------------------------------
#[cfg(feature = "ax-response")]
mod axum_impl {
    use super::AppError;
    use axum::response::{IntoResponse, Response};
    use http::StatusCode;
    use serde::Serialize;

    #[derive(Serialize)]
    struct ErrorBody<'a> {
        code: &'a str,
        message: String,
    }

    impl IntoResponse for AppError {
        fn into_response(self) -> Response {
            let (status, code) = match &self {
                AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
                AppError::TaskNotFound(_) => (StatusCode::NOT_FOUND, "task_not_found"),
                AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
                AppError::CircuitOpen(_) => (StatusCode::SERVICE_UNAVAILABLE, "circuit_open"),
                AppError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
                AppError::Redis(_) => (StatusCode::BAD_GATEWAY, "redis_error"),
                AppError::Llm(_) => (StatusCode::BAD_GATEWAY, "llm_error"),
                AppError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "config_error"),
                AppError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io_error"),
                AppError::Serde(_) => (StatusCode::BAD_REQUEST, "serde_error"),
                AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            };
            let body = ErrorBody {
                code,
                message: self.to_string(),
            };
            (status, axum::Json(body)).into_response()
        }
    }
}
