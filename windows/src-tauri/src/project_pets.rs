use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectPetsConfig {
    pub mappings: HashMap<String, String>,
}

pub struct ProjectPetStore {
    path: Option<PathBuf>,
    config: Mutex<ProjectPetsConfig>,
}

impl ProjectPetStore {
    pub fn new() -> Self {
        let path = dirs::config_dir().map(|d| d.join("AgentPet").join("project_pets.json"));
        let initial_config = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<ProjectPetsConfig>(&s).ok())
            .unwrap_or_default();

        Self {
            path,
            config: Mutex::new(initial_config),
        }
    }

    pub fn set_pet(&self, project: String, pet_id: String) -> HashMap<String, String> {
        let norm_proj = normalize_path(&project);
        let mut cfg = self.config.lock().unwrap();
        cfg.mappings.insert(norm_proj, pet_id);
        let map = cfg.mappings.clone();
        drop(cfg);
        self.save();
        map
    }

    pub fn remove_pet(&self, project: &str) -> HashMap<String, String> {
        let norm_proj = normalize_path(project);
        let mut cfg = self.config.lock().unwrap();
        cfg.mappings.remove(&norm_proj);
        let map = cfg.mappings.clone();
        drop(cfg);
        self.save();
        map
    }

    pub fn get_pet(&self, project: &str) -> Option<String> {
        let norm_proj = normalize_path(project);
        let cfg = self.config.lock().unwrap();
        cfg.mappings.get(&norm_proj).cloned()
    }

    pub fn get_all(&self) -> HashMap<String, String> {
        self.config.lock().unwrap().mappings.clone()
    }

    fn save(&self) {
        if let Some(ref p) = self.path {
            if let Some(dir) = p.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(cfg) = self.config.lock() {
                let _ = fs::write(p, serde_json::to_string_pretty(&*cfg).unwrap_or_default());
            }
        }
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_pet_store() {
        let store = ProjectPetStore::new();
        store.set_pet("E:\\workspace\\Project-A".into(), "cat".into());
        assert_eq!(store.get_pet("e:/workspace/project-a"), Some("cat".into()));
        assert_eq!(store.get_pet("E:/workspace/other"), None);
    }
}
