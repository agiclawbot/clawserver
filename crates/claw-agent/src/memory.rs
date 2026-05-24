//! 会话记忆抽象与具体实现。
//!
//! - `SessionStore` trait：脱离具体后端，便于测试注入与未来替换
//! - `RedisSessionStore`（生产）：基于 fred 的异步 Redis 会话记忆
//!   - `fred::RedisPool` 内置多连接异步池，单机/集群/哨兵由 URL 决定
//!   - 每次读写通过 pipeline 合并 RTT（LPUSH + LTRIM + EXPIRE）
//!   - 会话以 Redis List 存储消息 JSON，最新在前 (LPUSH)
//!   - 全异步、零锁，进程内不保留任何会话状态
//! - `InMemorySessionStore`（`#[cfg(test)]`）：进程内 HashMap，纯供测试。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fred::clients::RedisPool;
use fred::interfaces::{ClientLike, KeysInterface, ListInterface};
use fred::types::{
    Builder, ConnectionConfig, PerformanceConfig, RedisConfig as FredConfig,
};
use serde::{Deserialize, Serialize};

use claw_config::{AppConfig, RedisConfig};
use claw_core::chat::{ChatMessage, ChatRole};
use claw_core::error::{AppError, AppResult};

/// 会话存储抽象。生产用 `RedisSessionStore`，测试用 `InMemorySessionStore`。
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// 读取最近 `max_turns` 轮历史，按时间正序返回。
    async fn load(
        &self,
        app_id: &str,
        user_id: &str,
        session_id: &str,
        max_turns: usize,
    ) -> AppResult<Vec<ChatMessage>>;

    /// 追加一对 user / assistant 消息。
    async fn append(
        &self,
        app_id: &str,
        user_id: &str,
        session_id: &str,
        user_msg: &ChatMessage,
        assistant_msg: &ChatMessage,
    ) -> AppResult<()>;

    /// 健康检查（readyz 用）。
    async fn health(&self) -> AppResult<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTurn {
    pub role: String,
    pub content: String,
}

impl From<&ChatMessage> for StoredTurn {
    fn from(m: &ChatMessage) -> Self {
        Self {
            role: match m.role {
                ChatRole::System => "system".into(),
                ChatRole::User => "user".into(),
                ChatRole::Assistant => "assistant".into(),
                ChatRole::Tool => "tool".into(),
            },
            content: m.content.clone(),
        }
    }
}

impl StoredTurn {
    fn into_chat(self) -> ChatMessage {
        match self.role.as_str() {
            "system" => ChatMessage::system(self.content),
            "assistant" => ChatMessage::assistant(self.content),
            "tool" => ChatMessage::tool("", self.content),
            _ => ChatMessage::user(self.content),
        }
    }
}

/// SessionMemory 是 RedisSessionStore 的类型别名，供外层统一引用。
pub type SessionMemory = RedisSessionStore;

pub struct RedisSessionStore {
    pool: RedisPool,
    prefix: String,
    ttl_secs: i64,
    max_turns: usize,
}

impl RedisSessionStore {
    pub async fn connect(cfg: &AppConfig) -> AppResult<Arc<Self>> {
        let rcfg: &RedisConfig = &cfg.redis;
        if rcfg.urls.is_empty() {
            return Err(AppError::Config("redis.urls empty".into()));
        }

        // 单 url => fred 自动识别（redis-cluster://host1,host2 视为集群）
        let config = if rcfg.urls.len() == 1 {
            FredConfig::from_url(&rcfg.urls[0])
                .map_err(|e| AppError::Redis(format!("parse url: {e}")))?
        } else {
            // 多 url 视为集群：按 fred 约定拼装
            let joined = rcfg
                .urls
                .iter()
                .map(|u| u.trim_start_matches("redis://").trim_start_matches("rediss://"))
                .collect::<Vec<_>>()
                .join(",");
            FredConfig::from_url_clustered(&format!("redis-cluster://{}", joined))
                .map_err(|e| AppError::Redis(format!("parse cluster urls: {e}")))?
        };

        let perf = PerformanceConfig {
            auto_pipeline: true,
            default_command_timeout: Duration::from_millis(rcfg.command_timeout_ms),
            ..Default::default()
        };
        let conn = ConnectionConfig {
            connection_timeout: Duration::from_millis(rcfg.connect_timeout_ms),
            internal_command_timeout: Duration::from_millis(rcfg.command_timeout_ms),
            ..Default::default()
        };

        let pool = Builder::from_config(config)
            .set_performance_config(perf)
            .set_connection_config(conn)
            .build_pool(rcfg.pool_size)
            .map_err(|e| AppError::Redis(format!("build pool: {e}")))?;

        let _ = pool
            .init()
            .await
            .map_err(|e| AppError::Redis(format!("init: {e}")))?;

        Ok(Arc::new(Self {
            pool,
            prefix: rcfg.session_prefix.clone(),
            ttl_secs: rcfg.session_ttl_secs as i64,
            max_turns: rcfg.memory_max_turns,
        }))
    }

    #[inline]
    fn key(&self, app_id: &str, user_id: &str, session_id: &str) -> String {
        let mut s = String::with_capacity(
            self.prefix.len() + app_id.len() + user_id.len() + session_id.len() + 3,
        );
        s.push_str(&self.prefix);
        s.push_str(app_id);
        s.push(':');
        s.push_str(user_id);
        s.push(':');
        s.push_str(session_id);
        s
    }

}

#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn load(
        &self,
        app_id: &str,
        user_id: &str,
        session_id: &str,
        max_turns: usize,
    ) -> AppResult<Vec<ChatMessage>> {
        if max_turns == 0 {
            return Ok(Vec::new());
        }
        let key = self.key(app_id, user_id, session_id);
        let want = ((max_turns * 2).min(self.max_turns * 2)) as i64;
        let items: Vec<String> = self
            .pool
            .lrange::<Vec<String>, _>(key, 0, want - 1)
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        // LPUSH 最新在前，反序即时间正序
        let mut out = Vec::with_capacity(items.len());
        for raw in items.into_iter().rev() {
            if let Ok(t) = serde_json::from_str::<StoredTurn>(&raw) {
                out.push(t.into_chat());
            }
        }
        Ok(out)
    }

    async fn append(
        &self,
        app_id: &str,
        user_id: &str,
        session_id: &str,
        user_msg: &ChatMessage,
        assistant_msg: &ChatMessage,
    ) -> AppResult<()> {
        let key = self.key(app_id, user_id, session_id);
        let user_raw = serde_json::to_string(&StoredTurn::from(user_msg))?;
        let asst_raw = serde_json::to_string(&StoredTurn::from(assistant_msg))?;

        // fred pipeline：一次 RTT 完成 LPUSH / LTRIM / EXPIRE
        let client = self.pool.next();
        let pipe = client.pipeline();
        let _: () = pipe
            .lpush::<(), _, _>(&key, vec![asst_raw, user_raw])
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        let _: () = pipe
            .ltrim::<(), _>(&key, 0, (self.max_turns * 2) as i64 - 1)
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        let _: () = pipe
            .expire::<(), _>(&key, self.ttl_secs)
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        let _: Vec<()> = pipe
            .all()
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn health(&self) -> AppResult<()> {
        let _: String = self
            .pool
            .next()
            .ping()
            .await
            .map_err(|e| AppError::Redis(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 测试用 InMemorySessionStore：进程内 HashMap，仅在 `cargo test` 时编译。
// ---------------------------------------------------------------------------
#[cfg(test)]
pub use in_memory::InMemorySessionStore;

#[cfg(test)]
mod in_memory {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use tokio::sync::RwLock;

    /// 进程内会话存储（仅供测试）。
    /// 行为对齐 `RedisSessionStore`：append 头插（最新在前），load 反序输出（时间正序）。
    pub struct InMemorySessionStore {
        prefix: String,
        max_turns: usize,
        inner: RwLock<HashMap<String, VecDeque<StoredTurn>>>,
    }

    impl Default for InMemorySessionStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl InMemorySessionStore {
        pub fn new() -> Self {
            Self {
                prefix: "claw:sess:".into(),
                max_turns: 40,
                inner: RwLock::new(HashMap::new()),
            }
        }

        #[inline]
        fn key(&self, app_id: &str, user_id: &str, session_id: &str) -> String {
            format!("{}{}:{}:{}", self.prefix, app_id, user_id, session_id)
        }
    }

    #[async_trait]
    impl SessionStore for InMemorySessionStore {
        async fn load(
            &self,
            app_id: &str,
            user_id: &str,
            session_id: &str,
            max_turns: usize,
        ) -> AppResult<Vec<ChatMessage>> {
            if max_turns == 0 {
                return Ok(Vec::new());
            }
            let key = self.key(app_id, user_id, session_id);
            let g = self.inner.read().await;
            let q = match g.get(&key) {
                Some(q) => q,
                None => return Ok(Vec::new()),
            };
            let want = (max_turns * 2).min(self.max_turns * 2);
            let take = want.min(q.len());
            let mut out: Vec<ChatMessage> = Vec::with_capacity(take);
            // q 头部为最新；取前 take 个再反序得到时间正序
            for t in q.iter().take(take).rev() {
                out.push(t.clone().into_chat());
            }
            Ok(out)
        }

        async fn append(
            &self,
            app_id: &str,
            user_id: &str,
            session_id: &str,
            user_msg: &ChatMessage,
            assistant_msg: &ChatMessage,
        ) -> AppResult<()> {
            let key = self.key(app_id, user_id, session_id);
            let mut g = self.inner.write().await;
            let q = g.entry(key).or_default();
            // 与 Redis LPUSH [asst, user] 行为一致：user 最先入头，随后 asst 入头
            q.push_front(StoredTurn::from(assistant_msg));
            q.push_front(StoredTurn::from(user_msg));
            let cap = self.max_turns * 2;
            while q.len() > cap {
                q.pop_back();
            }
            Ok(())
        }

        async fn health(&self) -> AppResult<()> {
            Ok(())
        }
    }
}
