use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct PrefabFile {
    #[serde(default)]
    pub components: HashMap<String, Value>,
    /// Spawned as child entities (native flecs `(ChildOf, _)` → root).
    #[serde(default)]
    pub children: Vec<PrefabFile>,
}
