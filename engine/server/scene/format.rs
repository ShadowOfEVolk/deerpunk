use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct SceneFile {
    #[serde(default)]
    pub entities: Vec<SceneEntity>,
}

#[derive(Debug, Deserialize)]
pub struct SceneEntity {
    /// Prefab name without path/extension, e.g. `"main_menu"` → `prefabs/main_menu.prefab`.
    pub prefab: Option<String>,
    /// Inline / override components (merged over prefab).
    #[serde(default)]
    pub components: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct PrefabFile {
    #[serde(default)]
    pub components: HashMap<String, Value>,
}
