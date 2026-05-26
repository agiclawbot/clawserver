//! claw-cli 内部的最小 yaml 加载器。
//!
//! 设计：
//! - 仅解析当前 cli 子命令需要的字段（llm / tasks / skills 路径），不依赖根 crate
//! - 用 [`claw_llm::config::LlmConfig`] / [`CircuitBreakerConfig`] 直接做 sub-struct 反序列化
//! - 任务 / Skill 的字段只解析展示需要的几个，避免硬绑定根 crate 的完整 schema

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use claw_types::{CircuitBreakerConfig, LlmConfig};
use serde::{Deserialize, Serialize};

/// cli 自用的精简 AppConfig 视图。
///
/// 只保留与 cli 子命令相关的几段；其它字段一概忽略（serde 默认行为）。
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfigLite {
    pub llm: LlmConfig,
    pub circuit_breaker: CircuitBreakerConfig,
    /// 启动期由 cli 自己合并 tasks/*.yaml 进来；yaml 主文件不要求有此字段。
    #[serde(default)]
    pub tasks: HashMap<String, TaskConfigLite>,
}

/// task 的精简视图：cli 只关心 name/llm/prompt/mode/tools/skill。
#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfigLite {
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub llm: TaskLlmLite,
    pub prompt: PromptLite,
    #[serde(default)]
    pub mode: TaskModeLite,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskLlmLite {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_temp")]
    pub temperature: f32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptLite {
    #[serde(default)]
    pub system: String,
    pub user_template: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskModeLite {
    #[default]
    Plain,
    React,
}

fn default_true() -> bool {
    true
}
fn default_temp() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    0.9
}
fn default_max_tokens() -> u32 {
    2048
}
fn default_max_iterations() -> u32 {
    5
}

/// 加载 config_dir 下的 `config.yaml` + `tasks/*.yaml`，组装成精简 AppConfig。
pub fn load_app_config(dir: &Path) -> Result<AppConfigLite> {
    let main = dir.join("config.yaml");
    let raw = std::fs::read_to_string(&main)
        .with_context(|| format!("read {}", main.display()))?;
    let mut cfg: AppConfigLite = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse {}", main.display()))?;

    let tasks_dir = dir.join("tasks");
    if tasks_dir.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&tasks_dir)
            .with_context(|| format!("read_dir {}", tasks_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x, "yaml" | "yml"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        for p in entries {
            let raw = std::fs::read_to_string(&p)
                .with_context(|| format!("read {}", p.display()))?;
            let tc: TaskConfigLite = serde_yaml::from_str(&raw)
                .with_context(|| format!("parse {}", p.display()))?;
            if tc.enabled {
                cfg.tasks.insert(tc.name.clone(), tc);
            }
        }
    }

    validate(&cfg)?;
    Ok(cfg)
}

fn validate(cfg: &AppConfigLite) -> Result<()> {
    if cfg.llm.providers.is_empty() {
        return Err(anyhow!("llm.providers empty"));
    }
    if !cfg.llm.providers.contains_key(&cfg.llm.default_provider) {
        return Err(anyhow!(
            "llm.default_provider `{}` not in providers",
            cfg.llm.default_provider
        ));
    }
    for (name, t) in &cfg.tasks {
        if !cfg.llm.providers.contains_key(&t.llm.provider) {
            return Err(anyhow!(
                "task `{name}` references unknown provider `{}`",
                t.llm.provider
            ));
        }
    }
    Ok(())
}
