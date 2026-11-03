//! Stable `prefab_id` ↔ classname mapping (assigned at join via [`super::PrefabDefRpc`]).

use std::collections::HashMap;

/// Wire id for a registered prefab (`0` = invalid / unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrefabNetId(pub u16);

impl PrefabNetId {
    pub const INVALID: PrefabNetId = PrefabNetId(0);

    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// Server + client: resolve prefab classnames without repeating UTF-8 on every `StreamIn`.
#[derive(Default)]
pub struct PrefabNetRegistry {
    id_to_name: HashMap<u16, String>,
    name_to_id: HashMap<String, u16>,
}

impl PrefabNetRegistry {
    pub fn clear(&mut self) {
        self.id_to_name.clear();
        self.name_to_id.clear();
    }

    pub fn register(&mut self, id: PrefabNetId, classname: &str) -> Result<(), String> {
        if !id.is_valid() {
            return Err("prefab_id 0 is reserved".into());
        }
        let key = normalize_classname(classname);
        if let Some(existing) = self.name_to_id.get(&key) {
            if *existing != id.0 {
                return Err(format!(
                    "prefab '{key}' already registered as id {existing}, got {}",
                    id.0
                ));
            }
            return Ok(());
        }
        if self.id_to_name.contains_key(&id.0) {
            return Err(format!("prefab_id {} already used", id.0));
        }
        self.id_to_name.insert(id.0, key.clone());
        self.name_to_id.insert(key, id.0);
        Ok(())
    }

    pub fn id_of(&self, classname: &str) -> Option<PrefabNetId> {
        let key = normalize_classname(classname);
        self.name_to_id.get(&key).copied().map(PrefabNetId)
    }

    pub fn name_of(&self, id: PrefabNetId) -> Option<&str> {
        self.id_to_name.get(&id.0).map(|s| s.as_str())
    }

    /// Deterministic ids for connect init (sorted unique classnames).
    pub fn assign_ids_sorted(names: &[String]) -> Vec<(PrefabNetId, String)> {
        let mut uniq: Vec<String> = names
            .iter()
            .map(|n| normalize_classname(n))
            .collect();
        uniq.sort();
        uniq.dedup();
        uniq.into_iter()
            .enumerate()
            .map(|(i, name)| (PrefabNetId((i as u16) + 1), name))
            .collect()
    }
}

fn normalize_classname(classname: &str) -> String {
    classname
        .trim()
        .trim_end_matches(".prefab")
        .trim_end_matches(".jsonc")
        .to_string()
}
