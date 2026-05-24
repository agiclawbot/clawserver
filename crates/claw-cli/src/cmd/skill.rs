//! `clawctl skill` —— Skill 调试子命令。
//!
//! cli 内独立扫描 `<config_dir>/skills/<name>/{manifest.yaml, instruction.md}`，
//! 不引入根 crate，保持子命令模块独立。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;

use super::Ctx;
use crate::yaml_cfg;

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// 列出已加载 Skill
    List,
    /// 显示某 Skill 的 manifest + instruction
    Show {
        /// skill 名
        name: String,
    },
    /// 验证 skill 的 tools 白名单是否在 task 的工具集中（与配置交叉校验）
    Validate {
        /// skill 名
        name: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct SkillManifest {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tools: Vec<String>,
}

struct LoadedSkill {
    manifest: SkillManifest,
    instruction: String,
    dir: PathBuf,
}

pub async fn run(ctx: &Ctx, sub: Sub) -> Result<()> {
    let skills_dir = ctx.config_dir.join("skills");
    let skills = load_all(&skills_dir)?;

    match sub {
        Sub::List => {
            println!("{} skill(s) under {}:", skills.len(), skills_dir.display());
            let mut keys: Vec<&String> = skills.keys().collect();
            keys.sort();
            for k in keys {
                let s = skills.get(k).unwrap();
                println!(
                    "  {}{} - {} (tools={})",
                    s.manifest.name.green().bold(),
                    if s.manifest.version.is_empty() {
                        String::new()
                    } else {
                        format!("@{}", s.manifest.version)
                    },
                    if s.manifest.description.is_empty() {
                        "<no description>".to_string()
                    } else {
                        s.manifest.description.clone()
                    },
                    s.manifest.tools.len()
                );
            }
        }
        Sub::Show { name } => {
            let s = skills
                .get(&name)
                .ok_or_else(|| anyhow!("skill `{name}` not found in {}", skills_dir.display()))?;
            println!("{} {}", "name:".bold(), s.manifest.name);
            if !s.manifest.version.is_empty() {
                println!("{} {}", "version:".bold(), s.manifest.version);
            }
            if !s.manifest.description.is_empty() {
                println!("{} {}", "description:".bold(), s.manifest.description);
            }
            println!("{} {}", "dir:".bold(), s.dir.display());
            println!(
                "{} {}",
                "tools:".bold(),
                if s.manifest.tools.is_empty() {
                    "<none>".to_string()
                } else {
                    s.manifest.tools.join(", ")
                }
            );
            println!("{}", "instruction:".bold());
            if s.instruction.is_empty() {
                println!("  <empty>");
            } else {
                for line in s.instruction.lines() {
                    println!("  {line}");
                }
            }
        }
        Sub::Validate { name } => {
            let s = skills
                .get(&name)
                .ok_or_else(|| anyhow!("skill `{name}` not found"))?;
            // 与 config/tasks/*.yaml 中所有 task.tools 联合，作为已知工具集，做交叉校验
            let cfg = yaml_cfg::load_app_config(&ctx.config_dir).ok();
            let mut declared: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            if let Some(c) = &cfg {
                for t in c.tasks.values() {
                    for tool in &t.tools {
                        declared.insert(tool.as_str());
                    }
                }
            }
            let mut missing: Vec<&str> = Vec::new();
            for t in &s.manifest.tools {
                if !declared.contains(t.as_str()) {
                    missing.push(t.as_str());
                }
            }
            if missing.is_empty() {
                println!(
                    "{} all {} tool(s) declared in tasks",
                    "OK".green().bold(),
                    s.manifest.tools.len()
                );
            } else {
                println!(
                    "{} {} tool(s) not declared in any task: {}",
                    "WARN".yellow().bold(),
                    missing.len(),
                    missing.join(", ")
                );
                println!(
                    "  (note: skills 的 tools 是给 task 引用时用的白名单，需要 task.tools 中存在或 cli 注册表中加载)"
                );
            }
        }
    }
    Ok(())
}

fn load_all(root: &Path) -> Result<BTreeMap<String, LoadedSkill>> {
    let mut map = BTreeMap::new();
    if !root.is_dir() {
        return Ok(map);
    }
    for entry in std::fs::read_dir(root).with_context(|| format!("read_dir {}", root.display()))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("manifest.yaml");
        let raw = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let manifest: SkillManifest = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        let instruction =
            std::fs::read_to_string(dir.join("instruction.md")).unwrap_or_default();
        map.insert(
            manifest.name.clone(),
            LoadedSkill {
                manifest,
                instruction,
                dir,
            },
        );
    }
    Ok(map)
}
