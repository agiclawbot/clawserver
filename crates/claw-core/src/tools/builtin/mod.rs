//! 内置开箱即用的 Tool 实现。
//!
//! - [`TimeNow`]：无依赖，零参数返回当前 UTC 时间
//! - [`HttpGet`]：基于 reqwest 的安全 HTTP GET（受 `tools-http` feature 控制）
//! - [`WebSearch`]：占位 stub，方便上层接入 SerpAPI/Bing/Google CSE
//!
//! 全部实现 [`crate::tool::Tool`] trait；上层通过 `register(Arc::new(...))`
//! 装入 [`crate::tool::ToolRegistry`] 即可。

mod time_now;
mod web_search;

#[cfg(feature = "tools-http")]
mod http_get;

pub use time_now::TimeNow;
pub use web_search::WebSearch;

#[cfg(feature = "tools-http")]
pub use http_get::HttpGet;
