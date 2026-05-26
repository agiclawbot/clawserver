use std::collections::HashMap;
use std::sync::Arc;

use crate::skill::Skill;

#[derive(Default)]
pub struct SkillRegistry {
    items: HashMap<String, Arc<Skill>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    pub fn insert(&mut self, skill: Skill) {
        let name = skill.manifest.name.clone();
        self.items.insert(name, Arc::new(skill));
    }

    pub fn get(&self, name: &str) -> Option<Arc<Skill>> {
        self.items.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
