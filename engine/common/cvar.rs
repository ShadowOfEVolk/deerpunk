use std::collections::HashMap;
use std::sync::RwLock;

/// Регистрируется через `inventory::submit!` в любом крейте бинарника.
/// Автоматически собирается при `CvarSystem::initialize()`.
pub struct CvarDef {
    pub name: &'static str,
    pub default: &'static str,
    pub desc: &'static str,
}

inventory::collect!(CvarDef);

/// Runtime-репозиторий кваров. Значения — строки, парсятся на месте.
pub struct CvarSystem {
    initialized: bool,
    values: RwLock<HashMap<String, String>>,
}

impl CvarSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            values: RwLock::new(HashMap::new()),
        }
    }

    /// Собирает все `CvarDef` из inventory и применяет дефолты.
    pub fn initialize(&mut self) {
        self.initialized = true;
        let mut values = self.values.write().expect("cvars write lock");
        for def in inventory::iter::<CvarDef> {
            values.entry(def.name.to_string()).or_insert_with(|| def.default.to_string());
        }
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
        self.values.write().expect("cvars write lock").clear();
    }

    /// Установить значение квара. Возвращает `false` если кvar не найден.
    pub fn set(&self, name: &str, value: &str) -> bool {
        let mut map = self.values.write().expect("cvars write lock");
        if map.contains_key(name) {
            map.insert(name.to_string(), value.to_string());
            true
        } else {
            false
        }
    }

    /// Установить значение квара, создав его если нет (для .cfg где мог быть опечатан).
    pub fn set_or_create(&self, name: &str, value: &str) {
        self.values.write().expect("cvars write lock").insert(name.to_string(), value.to_string());
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.values.read().expect("cvars read lock").get(name).cloned()
    }

    pub fn get_f32(&self, name: &str) -> Option<f32> {
        self.values.read().expect("cvars read lock").get(name)?.parse().ok()
    }

    pub fn get_usize(&self, name: &str) -> Option<usize> {
        self.values.read().expect("cvars read lock").get(name)?.parse().ok()
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        let v = self.values.read().expect("cvars read lock").get(name)?.to_lowercase();
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

    /// Список всех зарегистрированных кваров с их текущими значениями.
    pub fn dump(&self) -> Vec<(String, String)> {
        let map = self.values.read().expect("cvars read lock");
        let mut list: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }
}

crate::singleton_system!(cvar, CvarSystem);

impl Default for CvarSystem {
    fn default() -> Self {
        Self::new()
    }
}
