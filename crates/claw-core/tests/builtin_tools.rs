//! 内置工具行为校验：time_now / web_search 通过 Tool trait 调用。
//! http_get 需要网络，在集成测试中跳过。

use claw_core::tool::Tool;
use claw_core::tools::builtin::{TimeNow, WebSearch};
use serde_json::{json, Value};

#[tokio::test]
async fn time_now_returns_unix_and_iso() {
    let t = TimeNow;
    assert_eq!(t.name(), "time_now");
    let raw = t.invoke(json!({})).await.unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    let unix = v.get("unix").and_then(Value::as_u64).expect("unix");
    let utc = v.get("utc").and_then(Value::as_str).expect("utc");
    assert!(unix > 1_700_000_000); // > 2023-11-14
    assert_eq!(utc.len(), 20);
    assert!(utc.ends_with('Z'));
    assert_eq!(&utc[10..11], "T");
}

#[test]
fn time_now_schema_is_empty_object() {
    let schema = TimeNow.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"].as_object().unwrap().is_empty());
    assert_eq!(schema["additionalProperties"], false);
}

#[tokio::test]
async fn web_search_stub_echoes_query_and_top_k() {
    let t = WebSearch;
    let raw = t.invoke(json!({"query": "rust async", "top_k": 5})).await.unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["query"], "rust async");
    assert_eq!(v["top_k"], 5);
    assert!(v["results"].is_array());
    assert!(v["note"].as_str().unwrap().contains("stub"));
}

#[tokio::test]
async fn web_search_default_top_k_is_three() {
    let t = WebSearch;
    let raw = t.invoke(json!({"query": "x"})).await.unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["top_k"], 3);
}

#[tokio::test]
async fn web_search_missing_query_is_bad_request() {
    let t = WebSearch;
    let res = t.invoke(json!({})).await;
    let err = match res {
        Ok(_) => panic!("expected error for missing query"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("query"));
}
