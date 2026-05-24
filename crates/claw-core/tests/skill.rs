//! claw-core skill loader / registry 行为校验。

use std::fs;

use claw_core::skill::{load_from_dir, SkillRegistry};
use tempfile::tempdir;

const MANIFEST_YAML: &str = r#"
name: math_helper
version: "0.1"
description: "math toolkit"
tools:
  - calc
  - time_now
defaults:
  model: "gpt-4o-mini"
  temperature: 0.2
  max_iterations: 4
"#;

const INSTRUCTION_MD: &str = "# Math Helper\n\nYou are a careful math tutor.\n";

#[test]
fn missing_dir_returns_empty_registry() {
    let dir = tempdir().unwrap();
    let ghost = dir.path().join("does-not-exist");
    let reg = load_from_dir(&ghost).expect("missing dir should be ok");
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
}

#[test]
fn empty_dir_returns_empty_registry() {
    let dir = tempdir().unwrap();
    let reg = load_from_dir(dir.path()).unwrap();
    assert!(reg.is_empty());
}

#[test]
fn loads_skill_with_manifest_and_instruction() {
    let dir = tempdir().unwrap();
    let skill_dir = dir.path().join("math_helper");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(skill_dir.join("manifest.yaml"), MANIFEST_YAML).unwrap();
    fs::write(skill_dir.join("instruction.md"), INSTRUCTION_MD).unwrap();

    let reg = load_from_dir(dir.path()).unwrap();
    assert_eq!(reg.len(), 1);
    let s = reg.get("math_helper").expect("found by name");
    assert_eq!(s.manifest.tools, vec!["calc", "time_now"]);
    assert_eq!(s.manifest.defaults.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(s.manifest.defaults.max_iterations, Some(4));
    assert!(s.instruction.contains("Math Helper"));
}

#[test]
fn missing_instruction_md_falls_back_to_empty() {
    let dir = tempdir().unwrap();
    let skill_dir = dir.path().join("math_helper");
    fs::create_dir(&skill_dir).unwrap();
    fs::write(skill_dir.join("manifest.yaml"), MANIFEST_YAML).unwrap();

    let reg = load_from_dir(dir.path()).unwrap();
    let s = reg.get("math_helper").unwrap();
    assert!(s.instruction.is_empty());
}

#[test]
fn malformed_skill_is_skipped_other_loaded() {
    let dir = tempdir().unwrap();
    let good = dir.path().join("good");
    fs::create_dir(&good).unwrap();
    fs::write(good.join("manifest.yaml"), MANIFEST_YAML.replace("math_helper", "good")).unwrap();

    let bad = dir.path().join("bad");
    fs::create_dir(&bad).unwrap();
    fs::write(bad.join("manifest.yaml"), ":\nnot: : valid").unwrap();

    let reg = load_from_dir(dir.path()).unwrap();
    assert_eq!(reg.len(), 1);
    assert!(reg.get("good").is_some());
    assert!(reg.get("bad").is_none());
}

#[test]
fn registry_default_constructs_empty() {
    let r = SkillRegistry::default();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}
