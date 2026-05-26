//! AgentRequest DTO 校验：必填字段 + 大小上限 + 严格 schema。

use claw_api::AgentRequest;
use serde_json::json;

fn good() -> serde_json::Value {
    json!({
        "app_id": "a",
        "user_id": "u",
        "session_id": "s",
        "task_type": "chat",
        "content": "hi",
    })
}

#[test]
fn parses_minimal_valid_request() {
    let req: AgentRequest = serde_json::from_value(good()).unwrap();
    assert_eq!(req.app_id, "a");
    assert_eq!(req.task_type, "chat");
    assert!(req.model.is_none());
    assert!(req.metadata.is_none());
    assert!(req.validate().is_ok());
}

#[test]
fn parses_with_optional_model_and_metadata() {
    let mut v = good();
    v["model"] = json!("gpt-4o-mini");
    v["metadata"] = json!({"trace_id": "x"});
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    assert_eq!(req.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(req.metadata.unwrap()["trace_id"], "x");
}

#[test]
fn deny_unknown_fields_rejects_typos() {
    let mut v = good();
    v["sesion_id"] = json!("typo"); // 多余字段
    let res: serde_json::Result<AgentRequest> = serde_json::from_value(v);
    assert!(res.is_err());
}

#[test]
fn validate_rejects_empty_required() {
    for field in ["app_id", "user_id", "session_id", "task_type", "content"] {
        let mut v = good();
        v[field] = json!("");
        let req: AgentRequest = serde_json::from_value(v).unwrap();
        let err = req.validate().unwrap_err();
        assert!(
            err.contains("required") || err.contains("invalid"),
            "field={field} err={err}"
        );
    }
}

#[test]
fn validate_rejects_whitespace_only() {
    let mut v = good();
    v["app_id"] = json!("   ");
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    assert!(req.validate().is_err());
}

#[test]
fn validate_rejects_field_too_long() {
    let mut v = good();
    v["app_id"] = json!("x".repeat(65));
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    let err = req.validate().unwrap_err();
    assert!(err.contains("too long"), "err={err}");
}

#[test]
fn validate_rejects_oversize_content() {
    let mut v = good();
    v["content"] = json!("x".repeat(513 * 1024));
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    let err = req.validate().unwrap_err();
    assert!(err.contains("too long"), "err={err}");
}

#[test]
fn validate_accepts_max_size_content() {
    let mut v = good();
    v["content"] = json!("x".repeat(512 * 1024));
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    assert!(req.validate().is_ok());
}

#[test]
fn validate_rejects_whitespace_model() {
    let mut v = good();
    v["model"] = json!("   ");
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    let err = req.validate().unwrap_err();
    assert!(err.contains("model invalid"), "err={err}");
}

#[test]
fn validate_rejects_model_too_long() {
    let mut v = good();
    v["model"] = json!("x".repeat(129));
    let req: AgentRequest = serde_json::from_value(v).unwrap();
    let err = req.validate().unwrap_err();
    assert!(err.contains("model invalid"), "err={err}");
}
