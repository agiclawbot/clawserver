//! API Key 认证 + 租户隔离。
//!
//! - 如果 `config/api_keys.yaml` 存在，开启认证模式
//! - 请求需携带 `Authorization: Bearer <key>` 头
//! - 无 api_keys.yaml 时运行在 open 模式（当前行为，兼容开发）

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::http::HeaderMap;
use axum::http::StatusCode;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct RawKeyEntry {
    key: String,
    tenant: String,
    #[serde(default)]
    apps: Vec<RawAppEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawAppEntry {
    id: String,
}

/// 认证成功后返回的租户信息。
#[derive(Debug, Clone)]
pub struct TenantInfo {
    pub tenant: String,
    /// 允许的 app_id 列表；空 = 全部放行。
    pub allowed_apps: Vec<String>,
}

// ---------------------------------------------------------------------------
// KeyStore
// ---------------------------------------------------------------------------

pub struct ApiKeyStore {
    entries: HashMap<String, (String, Vec<String>)>,
}

impl ApiKeyStore {
    /// 从 `config_dir/api_keys.yaml` 加载。
    ///
    /// - 文件不存在 → open 模式（不鉴权）
    /// - 解析失败 → warn + open 模式
    pub fn load(config_dir: &Path) -> Arc<Self> {
        let path = config_dir.join("api_keys.yaml");
        if !path.exists() {
            tracing::info!("api_keys.yaml not found, running in open mode");
            return Arc::new(Self { entries: HashMap::new() });
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to read api_keys.yaml: {e}, running in open mode");
                return Arc::new(Self { entries: HashMap::new() });
            }
        };

        let parsed: Vec<RawKeyEntry> = match serde_yaml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse api_keys.yaml: {e}, running in open mode");
                return Arc::new(Self { entries: HashMap::new() });
            }
        };

        let entries: HashMap<_, _> = parsed
            .into_iter()
            .map(|e| {
                let apps: Vec<String> = e.apps.into_iter().map(|a| a.id).collect();
                (e.key, (e.tenant, apps))
            })
            .collect();

        tracing::info!(count = entries.len(), "api_keys.yaml loaded, auth enabled");
        Arc::new(Self { entries })
    }

    pub fn enabled(&self) -> bool {
        !self.entries.is_empty()
    }

    /// 验证 Bearer token，返回租户信息。
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<TenantInfo, StatusCode> {
        if !self.enabled() {
            return Ok(TenantInfo {
                tenant: String::from("open"),
                allowed_apps: Vec::new(),
            });
        }

        let header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let key = header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let (tenant, apps) = self.entries.get(key).ok_or(StatusCode::UNAUTHORIZED)?;

        Ok(TenantInfo {
            tenant: tenant.clone(),
            allowed_apps: apps.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::path::Path;

    fn make_headers(key: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(k) = key {
            h.insert("authorization", format!("Bearer {k}").parse().unwrap());
        }
        h
    }

    #[test]
    fn open_mode_when_no_file() {
        let store = ApiKeyStore::load(Path::new("/nonexistent"));
        assert!(!store.enabled());
        let result = store.authenticate(&make_headers(None));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().tenant, "open");
    }

    #[test]
    fn open_mode_still_ok_with_any_header() {
        let store = ApiKeyStore::load(Path::new("/nonexistent"));
        let result = store.authenticate(&make_headers(Some("sk-anything")));
        assert!(result.is_ok());
    }

    #[test]
    fn empty_store_rejects_when_enabled() {
        let store = Arc::new(ApiKeyStore {
            entries: HashMap::new(),
        });
        // enabled() is false when entries empty — same as open mode
        assert!(!store.enabled());
    }

    #[test]
    fn authenticate_with_valid_key() {
        let mut entries = HashMap::new();
        entries.insert(
            "sk-test".into(),
            ("acme".into(), vec!["app-a".into(), "app-b".into()]),
        );
        let store = Arc::new(ApiKeyStore { entries });

        assert!(store.enabled());
        let result = store.authenticate(&make_headers(Some("sk-test"))).unwrap();
        assert_eq!(result.tenant, "acme");
        assert_eq!(result.allowed_apps, vec!["app-a", "app-b"]);
    }

    #[test]
    fn authenticate_with_wrong_key_returns_401() {
        let mut entries = HashMap::new();
        entries.insert("sk-valid".into(), ("acme".into(), vec![]));
        let store = Arc::new(ApiKeyStore { entries });

        let result = store.authenticate(&make_headers(Some("sk-wrong")));
        assert!(result.is_err());
    }

    #[test]
    fn authenticate_missing_header_returns_401() {
        let mut entries = HashMap::new();
        entries.insert("sk-valid".into(), ("acme".into(), vec![]));
        let store = Arc::new(ApiKeyStore { entries });

        let result = store.authenticate(&make_headers(None));
        assert!(result.is_err());
    }

    #[test]
    fn authenticate_wrong_scheme_returns_401() {
        let mut entries = HashMap::new();
        entries.insert("sk-valid".into(), ("acme".into(), vec![]));
        let store = Arc::new(ApiKeyStore { entries });

        let mut h = HeaderMap::new();
        h.insert("authorization", "Basic xyz".parse().unwrap());
        let result = store.authenticate(&h);
        assert!(result.is_err());
    }
}
