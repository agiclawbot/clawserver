//! AppError → Axum `IntoResponse` 映射。
//!
//! 使用 newtype `ApiError` 绕过 Rust 孤儿规则（axum 与 claw-config 均为外部 crate）。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use claw_types::AppError;

/// Axum 兼容的 API 错误包装。
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::TaskNotFound(_) => (StatusCode::NOT_FOUND, "TASK_NOT_FOUND"),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"),
            AppError::CircuitOpen(_) => (StatusCode::SERVICE_UNAVAILABLE, "CIRCUIT_OPEN"),
            AppError::Redis(_) => (StatusCode::INTERNAL_SERVER_ERROR, "REDIS_ERROR"),
            AppError::Llm(_) => (StatusCode::BAD_GATEWAY, "LLM_ERROR"),
            AppError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "CONFIG_ERROR"),
            AppError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "IO_ERROR"),
            AppError::Serde(_) => (StatusCode::UNPROCESSABLE_ENTITY, "SERDE_ERROR"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
            AppError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "TIMEOUT"),
        };

        let body = json!({
            "error": code,
            "message": self.0.to_string(),
        });

        (status, Json(body)).into_response()
    }
}

/// Axum handler 返回的便捷别名。
pub type ApiResult<T> = Result<T, ApiError>;

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        ApiError(e)
    }
}
