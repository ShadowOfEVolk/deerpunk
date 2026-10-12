use std::collections::HashMap;

/// Hammer / BSP keyvalues captured at map spawn (`classname`, `origin`, …).
#[derive(Clone, Default)]
pub struct MapKv {
    pub kv: HashMap<String, String>,
}

impl MapKv {
    pub fn new(kv: HashMap<String, String>) -> Self {
        Self { kv }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.kv.get(key).map(String::as_str)
    }
}
