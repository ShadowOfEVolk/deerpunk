use std::collections::HashMap;
use std::sync::RwLock;

/// Saved to `configs/config.cfg` (GoldSrc `FCVAR_ARCHIVE`).
pub const ARCHIVE: u32 = 1 << 0;
/// Saved to `configs/video.cfg` (Xash `FCVAR_RENDERINFO`).
pub const RENDERINFO: u32 = 1 << 1;

pub struct CvarDef {
    pub name: &'static str,
    pub default: &'static str,
    pub desc: &'static str,
    pub flags: u32,
}

inventory::collect!(CvarDef);

pub struct CvarSystem {
    initialized: bool,
    values: RwLock<HashMap<String, String>>,
    flags: RwLock<HashMap<String, u32>>,
}

impl CvarSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            values: RwLock::new(HashMap::new()),
            flags: RwLock::new(HashMap::new()),
        }
    }

    pub fn initialize(&mut self) {
        self.initialized = true;
        let mut values = self.values.write().expect("cvars write lock");
        let mut flags = self.flags.write().expect("cvars flags lock");
        for def in inventory::iter::<CvarDef> {
            values
                .entry(def.name.to_string())
                .or_insert_with(|| def.default.to_string());
            flags.insert(def.name.to_string(), def.flags);
        }
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
        self.values.write().expect("cvars write lock").clear();
        self.flags.write().expect("cvars flags lock").clear();
    }

    pub fn set(&self, name: &str, value: &str) -> bool {
        let mut map = self.values.write().expect("cvars write lock");
        if map.contains_key(name) {
            map.insert(name.to_string(), value.to_string());
            true
        } else {
            false
        }
    }

    pub fn set_or_create(&self, name: &str, value: &str) {
        self.values
            .write()
            .expect("cvars write lock")
            .insert(name.to_string(), value.to_string());
        // User-created cvars are not archived unless already flagged via CvarDef.
        let mut flags = self.flags.write().expect("cvars flags lock");
        flags.entry(name.to_string()).or_insert(0);
    }

    pub fn flags_of(&self, name: &str) -> u32 {
        self.flags
            .read()
            .expect("cvars flags lock")
            .get(name)
            .copied()
            .unwrap_or(0)
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.values.read().expect("cvars read lock").get(name).cloned()
    }

    pub fn get_f32(&self, name: &str) -> Option<f32> {
        self.values
            .read()
            .expect("cvars read lock")
            .get(name)?
            .parse()
            .ok()
    }

    pub fn get_usize(&self, name: &str) -> Option<usize> {
        self.values
            .read()
            .expect("cvars read lock")
            .get(name)?
            .parse()
            .ok()
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        let v = self
            .values
            .read()
            .expect("cvars read lock")
            .get(name)?
            .to_lowercase();
        match v.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    pub fn get_f32_or(&self, name: &str, default: f32) -> f32 {
        self.get_f32(name).unwrap_or(default)
    }

    pub fn get_usize_or(&self, name: &str, default: usize) -> usize {
        self.get_usize(name).unwrap_or(default)
    }

    pub fn get_bool_or(&self, name: &str, default: bool) -> bool {
        self.get_bool(name).unwrap_or(default)
    }

    pub fn dump(&self) -> Vec<(String, String)> {
        let map = self.values.read().expect("cvars read lock");
        let mut list: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }

    /// Cvars whose flags include `mask` (name, value), sorted by name.
    pub fn dump_flagged(&self, mask: u32) -> Vec<(String, String)> {
        let values = self.values.read().expect("cvars read lock");
        let flags = self.flags.read().expect("cvars flags lock");
        let mut list: Vec<_> = values
            .iter()
            .filter(|(name, _)| flags.get(*name).copied().unwrap_or(0) & mask != 0)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }
}

impl Default for CvarSystem {
    fn default() -> Self {
        Self::new()
    }
}
