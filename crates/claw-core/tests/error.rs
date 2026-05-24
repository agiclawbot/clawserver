//! AppError 转换 / 序列化路径测试。

use claw_core::error::AppError;

#[test]
fn from_serde_json_error_maps_to_serde_variant() {
    let bad: serde_json::Result<serde_json::Value> = serde_json::from_str("{not json");
    let err: AppError = bad.unwrap_err().into();
    assert!(matches!(err, AppError::Serde(_)));
    assert!(err.to_string().contains("serde error"));
}

#[test]
fn from_io_error_maps_to_io_variant() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let err: AppError = io.into();
    assert!(matches!(err, AppError::Io(_)));
    assert!(err.to_string().contains("io error"));
}

#[test]
fn display_messages_carry_context() {
    assert_eq!(
        AppError::TaskNotFound("chat".into()).to_string(),
        "task `chat` not found"
    );
    assert_eq!(AppError::RateLimited.to_string(), "rate limited");
    assert_eq!(AppError::Timeout.to_string(), "timeout");
    assert_eq!(
        AppError::CircuitOpen("openai").to_string(),
        "upstream circuit open: openai"
    );
}
