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
//! | `MEMORY.md` | **长期事实库**（Agent 在对话中积累更新） | "用户偏好流式响应" |
//! | `USER.md`  | 用户画像 | "用户是后端开发者，熟悉 axum" |
//!
//! 管理后台通过 `UserMemoryStore`（Arc + RwLock）实现运行时增删改，无需重启。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;

use claw_types::{AppError, AppResult};

/// 运行期共享的用户记忆存储。
pub type UserMemoryStore = Arc<RwLock<HashMap<String, UserMemory>>>;

/// 允许通过管理 API 读写的文件名集合。
pub const ALLOWED_FILES: &[&str] = &["AGENT.md", "SOULD.md", "RULES.md", "MEMORY.md", "USER.md"];

/// 单个用户的记忆文件内容（序列化为字符串）。
#[derive(Debug, Clone, Default)]
pub struct UserMemory {
    pub agent: Option<String>,
    pub soul: Option<String>,
    pub rules: Option<String>,
    pub memory: Option<String>,
    pub user_profile: Option<String>,
}

impl UserMemory {
    pub fn to_prompt(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(5);
        if let Some(ref s) = self.agent { parts.push(s.as_str()); }
        if let Some(ref s) = self.soul { parts.push(s.as_str()); }
        if let Some(ref s) = self.rules { parts.push(s.as_str()); }
        if let Some(ref s) = self.user_profile { parts.push(s.as_str()); }
        if let Some(ref s) = self.memory { parts.push(s.as_str()); }
        parts.join("\n\n---\n\n")
    }

    pub fn set_file(&mut self, name: &str, content: String) -> Option<String> {
        let old = match name {
            "AGENT.md" => std::mem::replace(&mut self.agent, Some(content)),
            "SOULD.md" => std::mem::replace(&mut self.soul, Some(content)),
            "RULES.md" => std::mem::replace(&mut self.rules, Some(content)),
            "MEMORY.md" => std::mem::replace(&mut self.memory, Some(content)),
            "USER.md" => std::mem::replace(&mut self.user_profile, Some(content)),
            _ => return None,
        };
        old.filter(|s| !s.is_empty())
    }

    pub fn get_file(&self, name: &str) -> Option<&str> {
        match name {
            "AGENT.md" => self.agent.as_deref(),
            "SOULD.md" => self.soul.as_deref(),
            "RULES.md" => self.rules.as_deref(),
            "MEMORY.md" => self.memory.as_deref(),
            "USER.md" => self.user_profile.as_deref(),
            _ => None,
        }
    }

    pub fn remove_file(&mut self, name: &str) -> Option<String> {
        let old = match name {
            "AGENT.md" => self.agent.take(),
            "SOULD.md" => self.soul.take(),
            "RULES.md" => self.rules.take(),
            "MEMORY.md" => self.memory.take(),
            "USER.md" => self.user_profile.take(),
            _ => return None,
        };
        old.filter(|s| !s.is_empty())
    }
}

// ===================== 加载 =====================

pub fn load_all_users(users_dir: &Path) -> AppResult<UserMemoryStore> {
    let mut map = HashMap::new();

    if !users_dir.is_dir() {
        return Ok(Arc::new(RwLock::new(map)));
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
            agent: read_file(&dir.join("AGENT.md")),
            soul: read_file(&dir.join("SOULD.md")),
            rules: read_file(&dir.join("RULES.md")),
            memory: read_file(&dir.join("MEMORY.md")),
            user_profile: read_file(&dir.join("USER.md")),
        };

        if memory.agent.is_some()
            || memory.soul.is_some()
            || memory.rules.is_some()
            || memory.memory.is_some()
            || memory.user_profile.is_some()
        {
            map.insert(user_id, memory);
        }
    }

    Ok(Arc::new(RwLock::new(map)))
}

// ===================== 运行时更新 =====================

pub async fn write_user_file(
    store: &UserMemoryStore,
    users_dir: &Path,
    user_id: &str,
    file_name: &str,
    content: &str,
) -> AppResult<()> {
    if !ALLOWED_FILES.contains(&file_name) {
        return Err(AppError::BadRequest(format!(
            "invalid file name `{file_name}`, allowed: {ALLOWED_FILES:?}"
        )));
    }

    let user_dir = users_dir.join(user_id);
    tokio::fs::create_dir_all(&user_dir)
        .await
        .map_err(|e| AppError::Io(e))?;
    tokio::fs::write(user_dir.join(file_name), content)
        .await
        .map_err(|e| AppError::Io(e))?;

    let mut map = store.write().await;
    let memory = map.entry(user_id.to_string()).or_default();
    memory.set_file(file_name, content.to_string());

    Ok(())
}

pub async fn delete_user_file(
    store: &UserMemoryStore,
    users_dir: &Path,
    user_id: &str,
    file_name: &str,
) -> AppResult<()> {
    if !ALLOWED_FILES.contains(&file_name) {
        return Err(AppError::BadRequest(format!(
            "invalid file name `{file_name}`, allowed: {ALLOWED_FILES:?}"
        )));
    }

    let disk_path = users_dir.join(user_id).join(file_name);
    let _ = tokio::fs::remove_file(&disk_path).await;

    let mut map = store.write().await;
    if let Some(memory) = map.get_mut(user_id) {
        memory.remove_file(file_name);
        let has_any = memory.agent.is_some()
            || memory.soul.is_some()
            || memory.rules.is_some()
            || memory.memory.is_some()
            || memory.user_profile.is_some();
        if !has_any {
            map.remove(user_id);
        }
    }

    Ok(())
}

// ===================== 内部辅助 =====================

fn read_file(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    std::fs::read_to_string(path).ok().filter(|s| !s.trim().is_empty())
}

// ===================== 测试 =====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn load_for_test(users_dir: &Path) -> HashMap<String, UserMemory> {
        let store = load_all_users(users_dir).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let guard = rt.block_on(store.read());
        guard.clone()
    }

    #[test]
    fn load_all_users_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let map = load_for_test(dir.path());
        assert!(map.is_empty());
    }

    #[test]
    fn load_all_users_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let users_dir = dir.path().join("users");
        fs::create_dir_all(users_dir.join("u001")).unwrap();
        fs::write(users_dir.join("u001/AGENT.md"), "你是 Rust 专家").unwrap();
        fs::write(users_dir.join("u001/SOULD.md"), "回答要简洁").unwrap();
        fs::write(users_dir.join("u001/MEMORY.md"), "用户偏好流式响应").unwrap();

        let map = load_for_test(&users_dir);
        assert_eq!(map.len(), 1);
        let u = map.get("u001").unwrap();
        assert_eq!(u.agent.as_deref(), Some("你是 Rust 专家"));
        assert_eq!(u.soul.as_deref(), Some("回答要简洁"));
        assert_eq!(u.memory.as_deref(), Some("用户偏好流式响应"));
        assert!(u.rules.is_none());
    }

    #[test]
    fn empty_user_dir_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let users_dir = dir.path().join("users");
        fs::create_dir_all(users_dir.join("u001")).unwrap();
        let map = load_for_test(&users_dir);
        assert!(map.is_empty());
    }

    #[test]
    fn to_prompt_joins_parts() {
        let m = UserMemory {
            agent: Some("# Agent\n你是助手".into()),
            soul: Some("## Soul\n友好".into()),
            rules: None,
            memory: Some("## 记忆\n用户要中文回答".into()),
            user_profile: None,
        };
        let prompt = m.to_prompt();
        assert!(prompt.contains("# Agent\n你是助手"));
        assert!(prompt.contains("## Soul\n友好"));
        assert!(prompt.contains("## 记忆\n用户要中文回答"));
        assert!(prompt.contains("---"));
    }

    #[test]
    fn set_get_remove_file() {
        let mut m = UserMemory::default();
        assert!(m.agent.is_none());
        m.set_file("AGENT.md", "你是专家".into());
        assert_eq!(m.get_file("AGENT.md"), Some("你是专家"));
        m.remove_file("AGENT.md");
        assert!(m.agent.is_none());
    }

    #[test]
    fn invalid_file_name_returns_none() {
        let mut m = UserMemory::default();
        assert!(m.set_file("EVIL.md", "x".into()).is_none());
        assert!(m.get_file("EVIL.md").is_none());
        assert!(m.remove_file("EVIL.md").is_none());
    }
}
