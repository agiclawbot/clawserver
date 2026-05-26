mod registry;

mod loader;

pub use loader::load_from_dir;
pub use registry::SkillRegistry;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Skill {
    pub manifest: SkillManifest,
    pub instruction: String,
}

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
