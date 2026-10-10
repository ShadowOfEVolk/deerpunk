use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use json_comments::StripComments;

use super::entity_def::EntityDef;

#[derive(Debug)]
pub enum PrefabResolveError {
    NotFound(PathBuf),
    Io(String),
    Parse(String),
}

impl std::fmt::Display for PrefabResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "prefab not found: {}", p.display()),
            Self::Io(e) => write!(f, "prefab io: {e}"),
            Self::Parse(e) => write!(f, "prefab parse: {e}"),
        }
    }
}

/// Cached map-entity prefabs (`prefabs/{classname}.prefab`).
#[derive(Default)]
pub struct PrefabManager {
    defs: HashMap<String, EntityDef>,
}

impl PrefabManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entity_path(gamedir: &Path, classname: &str) -> PathBuf {
        let id = classname
            .trim()
            .trim_end_matches(".prefab")
            .trim_end_matches(".jsonc");
        gamedir.join("prefabs").join(format!("{id}.prefab"))
    }

    pub fn exists_on_disk(gamedir: &Path, classname: &str) -> bool {
        Self::entity_path(gamedir, classname).is_file()
    }

    pub fn resolve(
        &mut self,
        gamedir: &Path,
        classname: &str,
    ) -> Result<EntityDef, PrefabResolveError> {
        if let Some(def) = self.defs.get(classname) {
            return Ok(def.clone());
        }
        let path = Self::entity_path(gamedir, classname);
        if !path.is_file() {
            return Err(PrefabResolveError::NotFound(path));
        }
        let file = File::open(&path).map_err(|e| PrefabResolveError::Io(format!("{}: {e}", path.display())))?;
        let reader = StripComments::new(BufReader::new(file));
        let mut def: EntityDef = serde_json::from_reader(reader)
            .map_err(|e| PrefabResolveError::Parse(format!("{}: {e}", path.display())))?;
        if def.id.is_empty() {
            def.id = classname.to_string();
        }
        self.defs.insert(classname.to_string(), def.clone());
        Ok(def)
    }
}
