//! Skill 加载器：扫描 `config/skills/<name>/{manifest.yaml, instruction.md}` 装入 Registry。

use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::skill::{Skill, SkillManifest, SkillRegistry};

/// 从给定根目录加载所有 skill。
///
/// - 目录结构：`<root>/<skill_name>/manifest.yaml` + `<root>/<skill_name>/instruction.md`
/// - 缺 instruction.md 时退化为空字符串（仍可工作）
/// - 子目录读取失败仅警告，不影响其他 skill 加载
pub fn load_from_dir(root: &Path) -> AppResult<SkillRegistry> {
    let mut reg = SkillRegistry::new();
    if !root.is_dir() {
        tracing::info!(dir = %root.display(), "skills dir not found, skill registry empty");
        return Ok(reg);
    }
    for entry in std::fs::read_dir(root)
        .map_err(|e| AppError::Config(format!("read {}: {}", root.display(), e)))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(err = %e, "skip malformed skill dir entry");
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_one(&path) {
            Ok(skill) => {
                tracing::info!(skill = %skill.manifest.name, "skill loaded");
                reg.insert(skill);
            }
            Err(e) => {
                tracing::warn!(dir = %path.display(), err = %e, "skill load failed, skipped");
            }
        }
    }
    Ok(reg)
}

fn load_one(dir: &Path) -> AppResult<Skill> {
    let manifest_path = dir.join("manifest.yaml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| AppError::Config(format!("read {}: {}", manifest_path.display(), e)))?;
    let manifest: SkillManifest = serde_yaml::from_str(&raw)?;

    let instruction_path = dir.join("instruction.md");
    let instruction = std::fs::read_to_string(&instruction_path).unwrap_or_default();

    Ok(Skill {
        manifest,
        instruction,
    })
}
