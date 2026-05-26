//! 只读任务注册表（零锁查询）。

use std::collections::HashMap;
use std::sync::Arc;

use claw_types::{AppConfig, TaskConfig};

pub struct TaskRegistry {
    inner: HashMap<String, Arc<TaskConfig>>,
}

impl TaskRegistry {
    pub fn build(cfg: &AppConfig) -> Arc<Self> {
        let mut m = HashMap::with_capacity(cfg.tasks.len());
        for (k, v) in &cfg.tasks {
            m.insert(k.clone(), Arc::new(v.clone()));
        }
        Arc::new(Self { inner: m })
    }

    #[inline]
    pub fn get(&self, task_type: &str) -> Option<Arc<TaskConfig>> {
        self.inner.get(task_type).cloned()
    }

    #[inline]
    pub fn contains(&self, task_type: &str) -> bool {
        self.inner.contains_key(task_type)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
