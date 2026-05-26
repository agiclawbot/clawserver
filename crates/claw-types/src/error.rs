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

impl From<serde_yaml::Error> for AppError {
    fn from(e: serde_yaml::Error) -> Self {
        AppError::Serde(e.to_string())
    }
}
