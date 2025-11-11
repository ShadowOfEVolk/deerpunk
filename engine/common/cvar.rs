pub struct CvarSystem {
    initialized: bool,
}

impl CvarSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
        }
    }

    pub fn initialize(&mut self) {
        self.initialized = true;
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
    }
}

crate::singleton_system!(cvar, CvarSystem);

impl Default for CvarSystem {
    fn default() -> Self {
        Self::new()
    }
}

