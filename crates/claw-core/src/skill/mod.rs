//! Skill（技能）体系。
//!
//! Skill = 一组工具 + 一段长 prompt + 一套工作流，相当于"技能包"：
//! - manifest.yaml：元数据（name / description / version / tools / defaults）
//! - instruction.md：长 system 指令文本（运行期会被拼到 system prompt 之前）
//!
//! 类型和注册表始终可用；loader（依赖 serde_yaml）仅在 `yaml` feature 开启时编译。

mod registry;

#[cfg(feature = "yaml")]
mod loader;

#[cfg(feature = "yaml")]
pub use loader::load_from_dir;

pub use registry::SkillRegistry;

use serde::Deserialize;

/// Skill 主体：manifest + instruction 合成的运行期对象。
#[derive(Debug, Clone)]
pub struct Skill {
    pub manifest: SkillManifest,
    /// 长指令文本（来自 instruction.md）。
    pub instruction: String,
}

/// `manifest.yaml` 反序列化结构。
#[derive(Debug, Clone, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub defaults: SkillDefaults,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillDefaults {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
}
