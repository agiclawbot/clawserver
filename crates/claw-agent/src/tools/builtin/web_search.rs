use async_trait::async_trait;
use serde_json::{json, Value};

use claw_types::{AppError, AppResult};
use claw_llm::Tool;

pub struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web and return short snippets. Use this when you need fresh \
         information that may not be in your training data."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "search keywords" },
                "top_k": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> AppResult<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::BadRequest("web_search: missing 'query'".into()))?;
        let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3);
        let stub = json!({
            "query": query,
            "top_k": top_k,
            "results": [],
            "note": "web_search stub: integrate SerpAPI/Bing/Google CSE"
        });
        Ok(stub.to_string())
    }
}

