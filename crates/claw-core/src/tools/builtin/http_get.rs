//! `http_get`：发起 HTTP GET 请求并返回正文（最多 4KB），用于 Agent 抓取网页。
//!
//! 安全考量：
//! - 仅允许 http/https；禁止 file://、ftp://、SSRF 类内网地址（10/172.16/192.168/127.0.0.1）
//! - 超时 8 秒；响应体上限 4KB（截断），避免被巨型页面打爆
//! - 仅返回正文文本前缀，不解析 HTML

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::tool::Tool;

const BODY_LIMIT: usize = 4 * 1024;
const REQUEST_TIMEOUT_SECS: u64 = 8;

pub struct HttpGet {
    client: reqwest::Client,
}

impl HttpGet {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("ClawServer-Agent/0.1")
            .build()
            .expect("build http client");
        Self { client }
    }
}

impl Default for HttpGet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HttpGet {
    fn name(&self) -> &str {
        "http_get"
    }

    fn description(&self) -> &str {
        "HTTP GET a public URL (http/https only) and return up to 4KB of response body. \
         Useful for fetching web pages or REST API responses. Internal IPs are blocked."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "absolute http(s) URL" }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> AppResult<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::BadRequest("http_get: missing 'url'".into()))?;

        if !is_safe_url(url) {
            return Err(AppError::BadRequest(format!(
                "http_get: unsafe url rejected: {url}"
            )));
        }

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("http_get send: {e}")))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("http_get body: {e}")))?;
        let mut body = String::from_utf8_lossy(&bytes).into_owned();
        if body.len() > BODY_LIMIT {
            body.truncate(BODY_LIMIT);
            body.push_str("\n...[truncated]");
        }
        Ok(json!({ "status": status.as_u16(), "body": body }).to_string())
    }
}

/// 仅放行 http/https 协议，禁止内网/loopback 主机以防 SSRF。
fn is_safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    const BLOCKED: &[&str] = &[
        "127.0.0.1", "localhost", "0.0.0.0",
        "10.", "192.168.", "169.254.",
        "172.16.", "172.17.", "172.18.", "172.19.",
        "172.20.", "172.21.", "172.22.", "172.23.",
        "172.24.", "172.25.", "172.26.", "172.27.",
        "172.28.", "172.29.", "172.30.", "172.31.",
    ];
    let host_part = lower
        .splitn(2, "://")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("");
    !BLOCKED.iter().any(|b| host_part.starts_with(b))
}
