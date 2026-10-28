use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Map-entity prefab (`prefabs/{classname}.prefab`).
/// Same shape as scene prefabs (`components`); optional `id`/`meta` for tooling.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EntityDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub icon_color: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    #[serde(default)]
    pub components: HashMap<String, Value>,
}
