//! ToolRegistry 注册 / 查找 / spec 白名单 / invoke 路径。

use std::sync::Arc;

use async_trait::async_trait;
use claw_core::error::AppResult;
use claw_core::tool::{Tool, ToolRegistry};
use serde_json::{json, Value};

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo back input"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn invoke(&self, args: Value) -> AppResult<String> {
        Ok(args.to_string())
    }
}

fn registry_with_echo() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(EchoTool));
    r
}

#[test]
fn register_and_get() {
    let r = registry_with_echo();
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());
    assert!(r.get("echo").is_some());
    assert!(r.get("missing").is_none());
}

#[test]
fn specs_for_filters_unknown_names() {
    let r = registry_with_echo();
    let specs = r.specs_for(&["echo".into(), "ghost".into()]);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].function.name, "echo");
    assert_eq!(specs[0].kind, "function");
}

#[test]
fn specs_for_empty_whitelist_yields_empty() {
    let r = registry_with_echo();
    assert!(r.specs_for(&[]).is_empty());
}

#[tokio::test]
async fn invoke_dispatches_by_name() {
    let r = registry_with_echo();
    let out = r.invoke("echo", json!({"a": 1})).await.unwrap();
    assert!(out.contains("\"a\":1"));
}

#[tokio::test]
async fn invoke_unknown_returns_internal_error() {
    let r = registry_with_echo();
    let err = r.invoke("ghost", json!({})).await.unwrap_err();
    assert!(err.to_string().contains("tool not registered"));
}
