//! # 用户分层记忆
//!
//! 每个用户在 `config/users/{user_id}/` 下可存放以下 Markdown 文件，
//! 启动时一次加载到内存，每次请求主动注入 system prompt：
//!
//! | 文件 | 用途 | 示例 |
//! |------|------|------|
//! | `AGENT.md` | 角色定义、操作手册 | "你是资深 Rust 工程师" |
//! | `SOULD.md` | 语气、行为规范 | "回答简洁专业，带代码示例" |
//! | `RULES.md` | 硬性规则 | "禁止使用 unsafe 代码" |
//! | `USER.md`  | 用户画像 | "用户是后端开发者，熟悉 axum" |
//!
//! 加载逻辑参考 `skill::load_from_dir`，启动期扫描 + 内存缓存，零运行时开销。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::error::AppResult;

/// 单个用户的记忆文件内容（序列化为字符串）。
///
/// 每个字段对应 `config/users/{user_id}/` 下的一个 Markdown 文件。
/// 文件不存在时对应字段为 `None`，不注入 prompt。
#[derive(Debug, Clone, Default)]
pub struct UserMemory {
    /// 角色定义 / 操作手册（AGENT.md）
    pub agent: Option<String>,
    /// 人格 / 语气 / 行为规范（SOULD.md）
    pub soul: Option<String>,
    /// 硬性规则（RULES.md）
    pub rules: Option<String>,
    /// 用户画像（USER.md）
    pub user_profile: Option<String>,
}

impl UserMemory {
    /// 将所有存在的记忆文件内容拼成一个字符串，用 Markdown 分隔。
    pub fn to_prompt(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        if let Some(ref s) = self.agent {
            parts.push(s.as_str());
        }
        if let Some(ref s) = self.soul {
            parts.push(s.as_str());
        }
        if let Some(ref s) = self.rules {
            parts.push(s.as_str());
        }
        if let Some(ref s) = self.user_profile {
            parts.push(s.as_str());
        }
        parts.join("\n\n---\n\n")
    }
}

/// 从 `config/users/` 目录加载所有用户的记忆文件。
///
/// 目录结构预期：
/// ```text
/// config/users/
/// ├── u001/
/// │   ├── AGENT.md
/// │   ├── SOULD.md
/// │   └── RULES.md
/// └── u002/
///     ├── AGENT.md
///     └── USER.md
/// ```
///
/// 目录不存在或为空时返回空 HashMap，不会报错。
pub fn load_all_users(users_dir: &Path) -> AppResult<Arc<HashMap<String, UserMemory>>> {
    let mut map = HashMap::new();

    if !users_dir.is_dir() {
        return Ok(Arc::new(map));
    }

    let mut entries: Vec<_> = std::fs::read_dir(users_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let user_id = entry.file_name().to_string_lossy().to_string();
        let dir = entry.path();

        let memory = UserMemory {
            agent: read_file_if_exists(&dir.join("AGENT.md")),
            soul: read_file_if_exists(&dir.join("SOULD.md")),
            rules: read_file_if_exists(&dir.join("RULES.md")),
            user_profile: read_file_if_exists(&dir.join("USER.md")),
        };

        // 至少有一个文件才录入，避免空目录
        if memory.agent.is_some()
            || memory.soul.is_some()
            || memory.rules.is_some()
            || memory.user_profile.is_some()
        {
            map.insert(user_id, memory);
        }
    }

    Ok(Arc::new(map))
}

/// 读取文件内容，不存在或读取失败时返回 `None`。
fn read_file_if_exists(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    std::fs::read_to_string(path).ok().filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_all_users_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let map = load_all_users(dir.path()).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn load_all_users_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let users_dir = dir.path().join("users");
        fs::create_dir_all(users_dir.join("u001")).unwrap();
        fs::write(users_dir.join("u001/AGENT.md"), "你是 Rust 专家").unwrap();
        fs::write(users_dir.join("u001/SOULD.md"), "回答要简洁").unwrap();

        let map = load_all_users(&users_dir).unwrap();
        assert_eq!(map.len(), 1);
        let u = map.get("u001").unwrap();
        assert_eq!(u.agent.as_deref(), Some("你是 Rust 专家"));
        assert_eq!(u.soul.as_deref(), Some("回答要简洁"));
        assert!(u.rules.is_none());
    }

    #[test]
    fn empty_user_dir_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let users_dir = dir.path().join("users");
        fs::create_dir_all(users_dir.join("u001")).unwrap(); // 空目录

        let map = load_all_users(&users_dir).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn to_prompt_joins_parts() {
        let m = UserMemory {
            agent: Some("# Agent\n你是助手".into()),
            soul: Some("## Soul\n友好".into()),
            rules: None,
            user_profile: None,
        };
        let prompt = m.to_prompt();
        assert!(prompt.contains("# Agent\n你是助手"));
        assert!(prompt.contains("## Soul\n友好"));
        assert!(prompt.contains("---"));
    }
}
